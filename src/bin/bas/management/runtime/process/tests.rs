#![cfg(any(unix, windows))]

use super::{Outcome, run};
#[cfg(unix)]
mod installation;
use std::{
    future::Future,
    io::{Read, Write},
    net::SocketAddr,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

const FIXTURE: &str = "management::runtime::process::tests::fixture";
const MODE: &str = "BAS_PROCESS_TEST_MODE";
const ROOT_ADDRESS: &str = "BAS_PROCESS_TEST_ROOT_ADDRESS";
const LEAF_ADDRESS: &str = "BAS_PROCESS_TEST_LEAF_ADDRESS";
const DEADLOCK_GUARD: Duration = Duration::from_secs(20);

fn fixture_command(mode: &str) -> std::process::Command {
    let mut command =
        std::process::Command::new(std::env::current_exe().expect("locate test executable"));
    command
        .args(["--exact", FIXTURE, "--nocapture"])
        .env(MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

// On assertion failure, socket closure also releases fixtures, independently of
// the process owner under test. Never detach a fixture from its inherited group.
struct FixtureChild(std::process::Child);

impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn fixture() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    assert!(mode == "root" || mode == "leaf");

    #[cfg(unix)]
    {
        // Only the subprocess changes its signal disposition. A TERM-only
        // implementation must not pass the tree-termination assertions.
        unsafe {
            nix::sys::signal::signal(
                nix::sys::signal::Signal::SIGTERM,
                nix::sys::signal::SigHandler::SigIgn,
            )
        }
        .expect("ignore TERM in fixture");
    }

    let address_key = if mode == "root" {
        ROOT_ADDRESS
    } else {
        LEAF_ADDRESS
    };
    let mut control =
        std::net::TcpStream::connect(std::env::var(address_key).expect("fixture control address"))
            .expect("connect fixture control");
    control
        .set_read_timeout(Some(DEADLOCK_GUARD * 3))
        .expect("bound fixture lifetime if controller fails");
    control
        .set_write_timeout(Some(DEADLOCK_GUARD))
        .expect("bound fixture writes");
    let _leaf = (mode == "root").then(|| {
        FixtureChild(
            fixture_command("leaf")
                .spawn()
                .expect("spawn leaf in inherited group/job"),
        )
    });
    control
        .write_all(&std::process::id().to_be_bytes())
        .expect("announce fixture readiness");
    loop {
        let mut message = [0];
        if control.read_exact(&mut message).is_err() {
            return;
        }
        match message[0] {
            b'p' => control.write_all(b"p").expect("reply to ping"),
            // process::exit deliberately leaves the leaf running and skips
            // FixtureChild's failure-cleanup guard.
            b'n' if mode == "root" => std::process::exit(0),
            b'x' if mode == "root" => std::process::exit(37),
            _ => panic!("unexpected fixture control message"),
        }
    }
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(DEADLOCK_GUARD, future)
        .await
        .expect("process test deadlock guard expired")
}

struct Peer {
    socket: TcpStream,
    pid: u32,
}

impl Peer {
    async fn accept(listener: &TcpListener) -> Self {
        let (mut socket, _) = bounded(listener.accept())
            .await
            .expect("accept fixture control connection");
        let pid = bounded(socket.read_u32())
            .await
            .expect("read fixture readiness PID");
        Self { socket, pid }
    }

    async fn send(&mut self, message: u8) {
        bounded(self.socket.write_all(&[message]))
            .await
            .expect("send fixture control message");
    }

    async fn ping(&mut self) {
        self.send(b'p').await;
        assert_eq!(
            bounded(self.socket.read_u8())
                .await
                .expect("fixture must answer ping"),
            b'p'
        );
    }

    async fn assert_closed(&mut self) {
        let mut byte = [0];
        let result = bounded(self.socket.read(&mut byte)).await;
        match result {
            Ok(0) => {}
            // Windows can report a reset rather than EOF on job termination.
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            other => panic!("fixture control must close on process exit: {other:?}"),
        }
    }
}

struct Attempt {
    task: JoinHandle<std::io::Result<Outcome>>,
    cancel: Option<oneshot::Sender<()>>,
    root: Peer,
    leaf: Peer,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Attempt {
    async fn start() -> Self {
        let root_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind root control listener");
        let leaf_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind leaf control listener");
        let mut command = fixture_command("root");
        command
            .env(ROOT_ADDRESS, address(&root_listener).to_string())
            .env(LEAF_ADDRESS, address(&leaf_listener).to_string());
        let (cancel, cancelled) = oneshot::channel();
        let task = tokio::spawn(run(command.into(), async move {
            let _ = cancelled.await;
        }));
        // Abort the owner even if readiness itself fails.
        let mut abort_on_failure = AbortOnDrop(Some(task.abort_handle()));
        let root = Peer::accept(&root_listener).await;
        let leaf = Peer::accept(&leaf_listener).await;
        abort_on_failure.0.take();
        Self {
            task,
            cancel: Some(cancel),
            root,
            leaf,
        }
    }

    fn cancel(&mut self) {
        self.cancel
            .take()
            .expect("attempt not previously cancelled")
            .send(())
            .expect("owner still awaiting cancellation");
    }

    async fn finish(&mut self) -> Outcome {
        let outcome = bounded(&mut self.task)
            .await
            .expect("owner task did not panic")
            .expect("owner completed without I/O error");
        assert_reaped(self.root.pid);
        self.root.assert_closed().await;
        self.leaf.assert_closed().await;
        outcome
    }
}

struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = &self.0 {
            handle.abort();
        }
    }
}

fn address(listener: &TcpListener) -> SocketAddr {
    listener
        .local_addr()
        .expect("read control listener address")
}

fn assert_reaped(pid: u32) {
    #[cfg(target_os = "linux")]
    {
        use nix::{
            errno::Errno,
            sys::wait::{Id, WaitPidFlag, waitid},
            unistd::Pid,
        };
        // WNOWAIT makes this assertion observational: it cannot reap a child
        // accidentally or mask the owner's missing wait.
        assert_eq!(
            waitid(
                Id::Pid(Pid::from_raw(pid.try_into().expect("PID fits pid_t"))),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
            ),
            Err(Errno::ECHILD),
            "owner must reap its direct child before returning"
        );
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

#[tokio::test]
async fn cancellation_terminates_root_and_leaf() {
    let mut attempt = Attempt::start().await;
    attempt.cancel();
    assert!(matches!(attempt.finish().await, Outcome::Cancelled));
}

#[tokio::test]
async fn normal_root_exit_terminates_residual_leaf() {
    let mut attempt = Attempt::start().await;
    attempt.leaf.ping().await;
    attempt.root.send(b'n').await;
    match attempt.finish().await {
        Outcome::Exited(status) => assert!(status.success()),
        Outcome::Cancelled => panic!("normal root exit was not cancellation"),
    }
}

#[tokio::test]
async fn parallel_attempts_have_independent_process_trees() {
    let (mut first, mut second) = tokio::join!(Attempt::start(), Attempt::start());
    first.cancel();
    assert!(matches!(first.finish().await, Outcome::Cancelled));
    second.root.ping().await;
    second.leaf.ping().await;
    second.cancel();
    assert!(matches!(second.finish().await, Outcome::Cancelled));
}
