use super::*;

fn target() -> Target {
    Target::Pipe(format!("bas-test-{}", uuid::Uuid::new_v4()))
}

#[test]
fn only_local_pipe_names_are_allowed() {
    for name in [
        "",
        ".",
        "..",
        "host/pipe",
        r"host\pipe",
        r"\\host\pipe\bas",
        "a\0b",
        "a:b",
    ] {
        assert_eq!(
            pipe_path(&Target::Pipe(name.into()))
                .expect_err("invalid name")
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
    assert!(pipe_path(&Target::Pipe("a".repeat(248))).is_err());
    assert!(pipe_path(&Target::Local).is_err());
    assert!(
        pipe_path(&Target::Tcp(
            "127.0.0.1:12345".parse().expect("socket address")
        ))
        .is_err()
    );
    assert_eq!(
        pipe_path(&Target::Pipe("bas-local".into())).expect("local name"),
        r"\\.\pipe\bas-local"
    );
}

#[test]
fn default_name_is_stable_and_user_scoped() {
    let target = default_target().expect("current user's target");
    assert_eq!(target, default_target().expect("same user's target"));
    assert_eq!(
        target,
        Target::Pipe(format!("bas-{}", current_user().expect("SID")))
    );
}

#[tokio::test]
async fn round_trip_and_name_ownership_survive_connection_drop() {
    let target = target();
    let mut listener = Listener::bind(&target).await.expect("bind");
    assert_eq!(listener.target(), &target);
    for _ in 0..2 {
        let (client, server) = tokio::join!(connect(&target), listener.accept());
        let mut client = client.expect("authenticated client");
        let mut server = server.expect("authenticated server");
        client.write_all(b"request").await.expect("write request");
        let mut bytes = [0; 7];
        server.read_exact(&mut bytes).await.expect("read request");
        assert_eq!(&bytes, b"request");
        server.write_all(b"reply").await.expect("write reply");
        let mut bytes = [0; 5];
        client.read_exact(&mut bytes).await.expect("read reply");
        assert_eq!(&bytes, b"reply");
    }
    assert!(
        Listener::bind(&target).await.is_err(),
        "listener still owns name"
    );
}

#[tokio::test]
async fn first_instance_rejects_existing_pipe_even_from_same_user() {
    let target = target();
    let listener = Listener::bind(&target).await.expect("first bind");
    assert!(
        Listener::bind(&target).await.is_err(),
        "collision must fail"
    );
    drop(listener);
    Listener::bind(&target)
        .await
        .expect("name released after drop");
}

#[tokio::test]
async fn bad_handshake_does_not_release_name() {
    let target = target();
    let mut listener = Listener::bind(&target).await.expect("bind");
    let mut client = ClientOptions::new()
        .open(pipe_path(&target).expect("path"))
        .expect("open");
    client.write_u8(0).await.expect("bad preface");
    assert!(listener.accept().await.is_err());
    assert!(Listener::bind(&target).await.is_err());
    let (client, server) = tokio::join!(connect(&target), listener.accept());
    assert!(client.is_ok());
    assert!(server.is_ok());
}

#[tokio::test]
async fn server_identity_is_checked_against_expected_sid() {
    let target = target();
    let _listener = Listener::bind(&target).await.expect("bind");
    let opened_after = system_time();
    let client = ClientOptions::new()
        .open(pipe_path(&target).expect("path"))
        .expect("open");
    verify_server(&client, &current_user().expect("SID"), opened_after).expect("same user");
    assert_eq!(
        verify_server(&client, "S-1-0-0", opened_after)
            .expect_err("different SID")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(
        verify_server(&client, &current_user().expect("SID"), 0).is_err(),
        "newer process rejected"
    );
}

#[tokio::test]
async fn cancelling_stalled_authentication_keeps_name_owned() {
    let target = target();
    let mut listener = Listener::bind(&target).await.expect("bind");
    let _silent_client = ClientOptions::new()
        .open(pipe_path(&target).expect("path"))
        .expect("open");
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    assert!(Listener::bind(&target).await.is_err());
    let (client, server) = tokio::join!(connect(&target), listener.accept());
    assert!(client.is_ok());
    assert!(server.is_ok());
}

#[tokio::test]
async fn busy_pipe_connection_has_a_deadline() {
    let target = target();
    let path = pipe_path(&target).expect("path");
    let _server = ServerOptions::new()
        .max_instances(1)
        .create(&path)
        .expect("single-instance pipe");
    let _occupied = ClientOptions::new().open(&path).expect("occupy pipe");
    let error = match connect(&target).await {
        Ok(_) => panic!("busy pipe must not connect"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn client_identity_comes_from_the_impersonation_token() {
    let target = target();
    let user = current_user().expect("SID");
    let mut server = create_pipe(&pipe_path(&target).expect("path"), &user, true).expect("pipe");
    let mut client = ClientOptions::new()
        .open(pipe_path(&target).expect("path"))
        .expect("open");
    client.write_u8(HANDSHAKE).await.expect("preface");
    server.connect().await.expect("accept");
    server.read_u8().await.expect("read preface");
    verify_client(&server, &user).expect("matching token");
    assert_eq!(
        verify_client(&server, "S-1-0-0")
            .expect_err("different expected SID")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    // A failed check must also restore the thread before any subsequent await.
    verify_client(&server, &user).expect("thread can authenticate again");
}
