use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::Gate;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct BlockedRead {
    released: Mutex<bool>,
    wake: Condvar,
}

impl BlockedRead {
    fn wait(&self) {
        let released = self.released.lock().expect("test mutex");
        let _released = self
            .wake
            .wait_while(released, |released| !*released)
            .expect("test condition");
    }

    fn release(&self) {
        *self.released.lock().expect("test mutex") = true;
        self.wake.notify_all();
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("condition completed");
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_drains_before_next_prompt_without_blocking_runtime() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("first session");
    let blocked = Arc::new(BlockedRead::default());
    let worker_blocked = blocked.clone();
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let mut read = Box::pin(session.read_with(
        move |cancelled| {
            worker_started.store(true, Ordering::Release);
            worker_blocked.wait();
            assert!(cancelled.load(Ordering::Acquire));
            Err(io::ErrorKind::Interrupted.into())
        },
        TEST_TIMEOUT,
    ));
    tokio::select! {
        result = &mut read => panic!("reader unexpectedly finished: {result:?}"),
        _ = until(|| started.load(Ordering::Acquire)) => {}
    }
    drop(read);
    drop(session);
    let next = gate.acquire();
    tokio::pin!(next);
    tokio::select! {
        _ = &mut next => panic!("old reader still owns stdin"),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    blocked.release();
    let next = tokio::time::timeout(TEST_TIMEOUT, next)
        .await
        .expect("drained old reader")
        .expect("next session");
    assert_eq!(
        next.read_with(|_| Ok("new answer".into()), TEST_TIMEOUT)
            .await
            .expect("new answer"),
        "new answer"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reusing_a_session_waits_for_its_cancelled_read_to_exit() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("session");
    let blocked = Arc::new(BlockedRead::default());
    let worker_blocked = blocked.clone();
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let mut first = Box::pin(session.read_with(
        move |_| {
            worker_started.store(true, Ordering::Release);
            worker_blocked.wait();
            Err(io::ErrorKind::Interrupted.into())
        },
        TEST_TIMEOUT,
    ));
    tokio::select! {
        _ = &mut first => panic!("first reader must block"),
        _ = until(|| started.load(Ordering::Acquire)) => {}
    }
    drop(first);
    let second_started = Arc::new(AtomicBool::new(false));
    let worker_started = second_started.clone();
    let second = session.read_with(
        move |_| {
            worker_started.store(true, Ordering::Release);
            Ok("next".into())
        },
        TEST_TIMEOUT,
    );
    tokio::pin!(second);
    tokio::select! {
        _ = &mut second => panic!("first reader still owns the session's read lease"),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    assert!(!second_started.load(Ordering::Acquire));
    blocked.release();
    assert_eq!(
        tokio::time::timeout(TEST_TIMEOUT, second)
            .await
            .expect("drained old read")
            .expect("new read"),
        "next"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_timeout_wakes_waiters_and_permanently_faults_stdin() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("session");
    let blocked = Arc::new(BlockedRead::default());
    let worker_blocked = blocked.clone();
    let mut read = Box::pin(session.read_with(
        move |_| {
            worker_blocked.wait();
            Ok("late result".into())
        },
        Duration::from_millis(20),
    ));
    assert!(futures::poll!(&mut read).is_pending());
    let waiter = gate.acquire();
    tokio::pin!(waiter);
    assert!(futures::poll!(&mut waiter).is_pending());
    drop(read);
    let result = tokio::time::timeout(TEST_TIMEOUT, waiter)
        .await
        .expect("fault wakes lease waiter");
    assert!(matches!(result, Err(error) if error.to_string().contains("stdin cannot be reused")));
    assert!(
        session
            .read_with(
                |_| panic!("faulted session must not spawn a reader"),
                TEST_TIMEOUT
            )
            .await
            .is_err()
    );
    blocked.release();
    until(|| Arc::strong_count(&blocked) == 1).await;
    drop(session);
    assert!(gate.acquire().await.is_err(), "fault cannot silently reset");
}

#[test]
fn dropping_read_signals_cancellation_even_if_runtime_immediately_shuts_down() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let gate = Arc::new(Gate::default());
    let (flag_send, flag_receive) = std::sync::mpsc::channel();
    let (exit_send, exit_receive) = std::sync::mpsc::channel();
    runtime.block_on(async {
        let session = gate.acquire().await.expect("session");
        let mut read = Box::pin(session.read_with(
            move |cancelled| {
                flag_send.send(cancelled.clone()).expect("flag observer");
                while !cancelled.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                exit_send.send(()).expect("exit observer");
                Err(io::ErrorKind::Interrupted.into())
            },
            TEST_TIMEOUT,
        ));
        assert!(futures::poll!(&mut read).is_pending());
        let flag = flag_receive
            .recv_timeout(TEST_TIMEOUT)
            .expect("worker started");
        drop(read);
        // No intervening await: the supervisor has not observed channel closure.
        assert!(
            flag.load(Ordering::Acquire),
            "Drop itself must set the flag"
        );
    });
    drop(runtime);
    exit_receive
        .recv_timeout(TEST_TIMEOUT)
        .expect("worker exits without a live runtime");
}

#[test]
fn runtime_shutdown_keeps_stdin_owned_until_worker_finishes_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let gate = Arc::new(Gate::default());
    let blocked = Arc::new(BlockedRead::default());
    let worker_blocked = blocked.clone();
    runtime.block_on(async {
        let session = gate.acquire().await.expect("session");
        let mut read = Box::pin(session.read_with(
            move |_| {
                worker_blocked.wait();
                Ok("done".into())
            },
            TEST_TIMEOUT,
        ));
        assert!(futures::poll!(&mut read).is_pending());
        drop(read);
    });
    drop(runtime);
    assert!(
        gate.lease.try_lock().is_err(),
        "worker owns stdin after supervisor shutdown"
    );
    blocked.release();
    let deadline = std::time::Instant::now() + TEST_TIMEOUT;
    while gate.lease.try_lock().is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "worker released lease"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn full_progress_queue_yields_then_drains_in_order_after_cancel() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("session");
    let blocked = Arc::new(BlockedRead::default());
    let worker_blocked = blocked.clone();
    let mut read = Box::pin(session.read_with(
        move |_| {
            worker_blocked.wait();
            Err(io::ErrorKind::Interrupted.into())
        },
        TEST_TIMEOUT,
    ));
    assert!(futures::poll!(&mut read).is_pending());
    let capacity = session
        .printer
        .sender()
        .capacity()
        .expect("bounded printer");
    let mut expected = Vec::new();
    for index in 0..capacity {
        let message = format!("progress {index}");
        session
            .progress_with(&message, |_| panic!("live reader owns output"))
            .await
            .expect("queued");
        expected.push(message);
    }
    let mut observed = Vec::new();
    let mut full = Box::pin(session.progress_with("last\x1b[2J", |line| observed.push(line)));
    assert!(futures::poll!(&mut full).is_pending());
    // The caller can still receive a terminal event while this future is pending.
    tokio::select! {
        _ = &mut full => panic!("printer is still full"),
        _ = tokio::task::yield_now() => {}
    }
    drop(read);
    blocked.release();
    tokio::time::timeout(TEST_TIMEOUT, &mut full)
        .await
        .expect("reader exit unblocks progress")
        .expect("printed");
    drop(full);
    expected.push("last\\u001b[2J".into());
    assert_eq!(observed, expected);
    let _reading = session.reading.lock().await;
    session.flush_progress(&mut |_| panic!("no duplicate after draining"));
}

#[tokio::test(flavor = "current_thread")]
async fn editor_consumed_progress_is_not_repeated_by_exit_drain() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("session");
    let reading = session.reading.lock().await;
    let editor = session.printer.clone();
    session
        .progress_with("already displayed", |_| panic!("must queue"))
        .await
        .expect("queued");
    assert_eq!(editor.get_line().as_deref(), Some("already displayed\n"));
    session
        .progress_with("pending at cancellation", |_| panic!("must queue"))
        .await
        .expect("queued");
    drop(editor);
    drop(reading);
    let mut observed = Vec::new();
    let _reading = session.reading.lock().await;
    session.flush_progress(&mut |line| observed.push(line));
    assert_eq!(observed, ["pending at cancellation"]);
}

#[tokio::test(flavor = "current_thread")]
async fn editor_and_exit_drain_preserve_empty_and_trailing_progress_lines() {
    let gate = Arc::new(Gate::default());
    let session = gate.acquire().await.expect("session");
    let editor = session.printer.clone();
    for message in [
        "",
        "\n",
        "first\n",
        "first\n\n",
        "first\nsecond",
        "\x1b[2J\n",
    ] {
        let safe = format!("{}", crate::safe::Safe(message));
        let reading = session.reading.lock().await;
        session
            .progress_with(message, |_| panic!("active editor owns output"))
            .await
            .expect("queued");
        let queued = editor.get_line().expect("progress for active editor");
        // This is the public printer payload consumed by Reedline's `lines()`
        // conversion. Each resulting line is painted, including empty ones.
        let displayed: String = queued.lines().map(|line| format!("{line}\n")).collect();
        assert_eq!(displayed, format!("{safe}\n"));

        session
            .progress_with(message, |_| panic!("must queue before reader exits"))
            .await
            .expect("queued for cancellation");
        drop(reading);
        let mut drained = Vec::new();
        let _reading = session.reading.lock().await;
        session.flush_progress(&mut |line| drained.push(line));
        assert_eq!(drained, [safe]);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rpc_error_and_eof_cancel_and_drain_before_escaping_elicitation() {
    use futures::StreamExt;
    for eof in [false, true] {
        let gate = Arc::new(Gate::default());
        let session = gate.acquire().await.expect("session");
        let started = Arc::new(AtomicBool::new(false));
        let worker_started = started.clone();
        let exited = Arc::new(AtomicBool::new(false));
        let worker_exited = exited.clone();
        let response = async {
            session
                .read_with(
                    move |cancelled| {
                        worker_started.store(true, Ordering::Release);
                        while !cancelled.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        worker_exited.store(true, Ordering::Release);
                        Err(io::ErrorKind::Interrupted.into())
                    },
                    TEST_TIMEOUT,
                )
                .await?;
            Ok(serde_json::json!({"action":"accept"}))
        };
        let mut events = futures::stream::once(async move {
            until(|| started.load(Ordering::Acquire)).await;
            if eof {
                None
            } else {
                Some(Err(tonic::Status::unavailable("lost connection")))
            }
        })
        .filter_map(futures::future::ready)
        .boxed();
        let result = super::super::await_elicitation(
            &session,
            response,
            "request",
            &mut events,
            &mut Default::default(),
        )
        .await;
        assert!(result.is_err());
        assert!(
            exited.load(Ordering::Acquire),
            "error must not bypass drain"
        );
        assert!(session.reading.try_lock().is_ok());
    }
}
