use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn endpoint() -> (tempfile::TempDir, Target) {
    let directory = tempfile::tempdir().expect("create test directory");
    fs::set_permissions(directory.path(), Permissions::from_mode(DIRECTORY_MODE))
        .expect("make test directory private");
    // macOS commonly exposes its temp directory through a symlink.
    let path = fs::canonicalize(directory.path()).expect("resolve test directory");
    (directory, Target::Unix(path.join("service.sock")))
}

#[test]
fn filesystem_root_is_the_namespace_trust_anchor() {
    // Nix sandbox roots can have an unmapped owner instead of numeric UID 0.
    validate_ancestors(Path::new("/")).expect("validate namespace root");
}

#[tokio::test]
async fn same_user_connects_with_restrictive_modes() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let mut listener = Listener::bind(&target).await.expect("bind socket");
    assert_eq!(listener.target(), &target);
    let mut client = connect(&target).await.expect("authenticate server");
    let mut server = listener.accept().await.expect("authenticate client");
    client.write_all(b"hello").await.expect("write client");
    let mut buffer = [0; 5];
    server.read_exact(&mut buffer).await.expect("read server");
    assert_eq!(&buffer, b"hello");
    assert_eq!(
        fs::metadata(&path).expect("socket metadata").mode() & 0o777,
        FILE_MODE
    );
    assert_eq!(
        fs::metadata(path.parent().expect("parent"))
            .expect("directory metadata")
            .mode()
            & 0o777,
        DIRECTORY_MODE
    );
    assert_eq!(
        fs::metadata(path.with_file_name("service.sock.lock"))
            .expect("lock metadata")
            .mode()
            & 0o777,
        FILE_MODE
    );
    drop(listener);
    assert!(!path.exists());
    assert!(path.with_file_name("service.sock.lock").exists());
}

#[tokio::test]
async fn live_listener_collision_preserves_endpoint() {
    let (_directory, target) = endpoint();
    let listener = Listener::bind(&target).await.expect("first listener");
    let error = Listener::bind(&target).await.err().expect("collision");
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    connect(&target)
        .await
        .expect("original listener still reachable");
    drop(listener);
    Listener::bind(&target)
        .await
        .expect("rebind after graceful stop");
}

#[tokio::test]
async fn stale_socket_is_recovered_but_unlocked_live_socket_is_not() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let old = std::os::unix::net::UnixListener::bind(&path).expect("unmanaged listener");
    fs::set_permissions(&path, Permissions::from_mode(FILE_MODE)).expect("socket permissions");
    assert_eq!(
        Listener::bind(&target)
            .await
            .err()
            .expect("live collision")
            .kind(),
        io::ErrorKind::AddrInUse
    );
    drop(old);
    let listener = Listener::bind(&target).await.expect("recover stale socket");
    drop(listener);
    assert!(!path.exists());
}

#[tokio::test]
async fn cleanup_does_not_remove_replacement() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let listener = Listener::bind(&target).await.expect("listener");
    fs::remove_file(&path).expect("unlink original");
    let replacement = std::os::unix::net::UnixListener::bind(&path).expect("replacement socket");
    drop(listener);
    assert!(path.exists());
    drop(replacement);
}

#[tokio::test]
async fn rejects_unsafe_directory_and_non_socket_endpoint() {
    let (directory, target) = endpoint();
    fs::set_permissions(directory.path(), Permissions::from_mode(0o755)).expect("unsafe mode");
    assert_eq!(
        Listener::bind(&target)
            .await
            .err()
            .expect("reject public directory")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    fs::set_permissions(directory.path(), Permissions::from_mode(DIRECTORY_MODE))
        .expect("private mode");
    let path = socket_path(&target).expect("socket path");
    fs::write(&path, b"keep me").expect("existing file");
    assert!(Listener::bind(&target).await.is_err());
    assert_eq!(fs::read(&path).expect("preserved file"), b"keep me");
    assert!(connect(&target).await.is_err());
}

#[tokio::test]
async fn rejects_symlinks_and_unsafe_lockfiles() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let destination = path.with_file_name("destination");
    fs::write(&destination, b"keep me").expect("destination");
    std::os::unix::fs::symlink(&destination, &path).expect("socket symlink");
    assert!(Listener::bind(&target).await.is_err());
    assert!(connect(&target).await.is_err());
    fs::remove_file(&path).expect("remove test symlink");
    let lock_path = path.with_file_name("service.sock.lock");
    fs::remove_file(&lock_path).expect("remove test lock");
    std::os::unix::fs::symlink(&destination, &lock_path).expect("lock symlink");
    assert!(Listener::bind(&target).await.is_err());
    assert_eq!(
        fs::read(destination).expect("preserved destination"),
        b"keep me"
    );
}

#[tokio::test]
async fn rejects_symlink_parent_and_world_writable_ancestor() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let private = path.with_file_name("private");
    DirBuilder::new()
        .mode(DIRECTORY_MODE)
        .create(&private)
        .expect("private directory");
    let alias = path.with_file_name("alias");
    std::os::unix::fs::symlink(&private, &alias).expect("directory symlink");
    assert!(
        Listener::bind(&Target::Unix(alias.join("service.sock")))
            .await
            .is_err()
    );
    fs::set_permissions(&private, Permissions::from_mode(0o777)).expect("unsafe ancestor");
    assert!(
        Listener::bind(&Target::Unix(private.join("child/service.sock")))
            .await
            .is_err()
    );
    assert!(!private.join("child").exists());
}

#[tokio::test]
async fn rejects_public_lockfile_without_changing_it() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let lock = path.with_file_name("service.sock.lock");
    fs::write(&lock, b"keep me").expect("lockfile");
    fs::set_permissions(&lock, Permissions::from_mode(0o644)).expect("public lock");
    assert!(Listener::bind(&target).await.is_err());
    assert_eq!(fs::read(&lock).expect("preserved lock"), b"keep me");
    assert_eq!(
        fs::metadata(lock).expect("lock metadata").mode() & 0o777,
        0o644
    );
}

#[tokio::test]
async fn creates_missing_final_parent_privately() {
    let (_directory, target) = endpoint();
    let path = socket_path(&target).expect("socket path");
    let target = Target::Unix(path.with_file_name("private").join("service.sock"));
    Listener::bind(&target)
        .await
        .expect("create private parent");
}

#[tokio::test]
async fn rejects_non_unix_targets() {
    for target in [
        Target::Local,
        Target::Pipe("bas-test".into()),
        Target::Tcp("127.0.0.1:1".parse().expect("TCP address")),
    ] {
        assert_eq!(
            Listener::bind(&target)
                .await
                .err()
                .expect("reject target")
                .kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            connect(&target).await.expect_err("reject target").kind(),
            io::ErrorKind::Unsupported
        );
    }
}
