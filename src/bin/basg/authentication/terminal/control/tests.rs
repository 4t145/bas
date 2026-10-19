use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

async fn pair() -> (Controller, Helper) {
    let listener = Listener::bind().expect("bind control listener");
    let (controller, helper) = timeout(TEST_TIMEOUT, async {
        tokio::join!(listener.accept(), listener.endpoint().connect())
    })
    .await
    .expect("control handshake completes");
    (
        controller.expect("accept authenticated helper"),
        helper.expect("connect authenticated helper"),
    )
}

#[tokio::test]
async fn rejects_missing_and_partial_results() {
    for count in 0..4 {
        let (mut controller, mut helper) = pair().await;
        helper
            .socket
            .write_all(&42_i32.to_be_bytes()[..count])
            .await
            .expect("send partial result");
        drop(helper);
        assert_eq!(
            controller
                .result()
                .await
                .expect_err("reject incomplete code")
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}

#[tokio::test]
async fn controller_drop_cancels_helper() {
    let (controller, mut helper) = pair().await;
    // Poll cancellation while the parent is still alive, before closing it.
    let mut cancelled = Box::pin(helper.cancelled());
    assert!(futures::poll!(&mut cancelled).is_pending());
    drop(controller);
    timeout(TEST_TIMEOUT, cancelled)
        .await
        .expect("controller closure wakes helper");
}

#[tokio::test]
async fn attempts_are_independent() {
    let (first, mut first_helper) = pair().await;
    let (mut second, mut second_helper) = pair().await;
    drop(first);
    timeout(TEST_TIMEOUT, first_helper.cancelled())
        .await
        .expect("first helper observes cancellation");
    {
        let mut cancelled = Box::pin(second_helper.cancelled());
        assert!(futures::poll!(&mut cancelled).is_pending());
    }
    second_helper
        .finish(23)
        .await
        .expect("finish second attempt");
    assert_eq!(second.result().await.expect("second result"), 23);
}

#[tokio::test]
async fn dropping_listener_prevents_helper_start() {
    let listener = Listener::bind().expect("bind listener");
    let endpoint = listener.endpoint().clone();
    // The TCP connection alone must never authorize starting login.
    let mut connecting = Box::pin(endpoint.connect());
    assert!(futures::poll!(&mut connecting).is_pending());
    drop(listener);
    assert!(
        timeout(TEST_TIMEOUT, connecting)
            .await
            .expect("closed listener terminates handshake")
            .is_err()
    );
}

#[tokio::test]
async fn rejects_non_loopback_endpoint() {
    let endpoint = Endpoint {
        address: "192.0.2.1:42".parse().expect("valid test address"),
        token: Uuid::new_v4(),
    };
    let error = endpoint
        .connect()
        .await
        .err()
        .expect("reject remote address");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
