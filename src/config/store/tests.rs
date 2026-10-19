use super::*;

#[cfg(unix)]
#[test]
fn save_lock_releases_even_while_a_spawned_process_inherits_a_descriptor() {
    let directory = tempfile::tempdir().expect("lock fixture");
    let path = directory.path().join("settings.lock");
    let file = fs::File::create(&path).expect("lock file");
    file.try_lock().expect("first owner");
    let inherited = file.try_clone().expect("inherited descriptor");
    let guard = WriteLock(file);
    let next = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("next writer");
    assert!(matches!(next.try_lock(), Err(fs::TryLockError::WouldBlock)));
    drop(guard);
    next.try_lock()
        .expect("prior save finished despite inherited descriptor");
    drop(inherited);
}

fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("temporary settings directory");
    let store = Store {
        path: dir.path().join("settings.json"),
        import_legacy: true,
    };
    (dir, store)
}

#[test]
fn broken_legacy_is_not_silently_discarded() {
    let (dir, store) = fixture();
    for (filename, value) in [
        ("appearance.json", r#"{"ui_font_size":0}"#),
        ("agents.json", "[]"),
        (
            "agents.json",
            r#"[{"id":"managed","installation":{"path":"/bas-installation.json","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}}]"#,
        ),
    ] {
        let path = dir.path().join(filename);
        fs::write(&path, value).expect("write invalid legacy");
        let error = store.load().expect_err("must reject legacy");
        assert!(
            error
                .to_string()
                .contains(path.to_str().expect("temp path"))
        );
        assert_eq!(
            fs::read_to_string(&path).expect("original preserved"),
            value
        );
        assert!(!store.path.exists());
        fs::remove_file(path).expect("remove fixture");
    }
}

#[test]
fn busy_invalid_and_io_failures_preserve_document_and_file() {
    let (_dir, store) = fixture();
    let mut document = store.load().expect("load");
    let before = document.settings.clone();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store.lock_path())
        .expect("lock file");
    lock.try_lock().expect("hold lock");
    assert!(matches!(
        store.save_appearance(&mut document, Appearance::default()),
        Err(Error::Busy { .. })
    ));
    drop(lock);
    fs::write(&store.path, r#"{"daemon":{"agents":[]}}"#).expect("invalid settings");
    let original = fs::read(&store.path).expect("bytes");
    assert!(
        store
            .save_appearance(&mut document, Appearance::default())
            .is_err()
    );
    assert_eq!(fs::read(&store.path).expect("unchanged"), original);
    assert_eq!(document.settings, before);
    fs::remove_file(&store.path).expect("remove fixture");
    fs::create_dir(&store.path).expect("unreadable as file");
    assert!(matches!(store.load(), Err(Error::Io { .. })));
    assert!(
        store
            .save_agents(&mut document, vec![AgentSpec::embedded().into()])
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn managed_links_are_readable_but_never_overwritten_or_treated_as_missing() {
    use std::os::unix::fs::symlink;
    let (dir, store) = fixture();
    let mut document = store.load().expect("load");
    let target = dir.path().join("target");
    symlink(&target, &store.path).expect("broken link");
    assert!(matches!(store.load(), Err(Error::Io { .. })));
    assert!(
        store
            .save_appearance(&mut document, Appearance::default())
            .is_err()
    );
    fs::write(&target, "{}").expect("target");
    assert_eq!(
        store.load().expect("read managed config").settings,
        Settings::default()
    );
    assert!(
        store
            .save_agents(&mut document, vec![AgentSpec::embedded().into()])
            .is_err()
    );
    assert_eq!(fs::read_to_string(&target).expect("target unchanged"), "{}");
    let linked_dir = dir.path().join("linked");
    symlink(dir.path(), &linked_dir).expect("parent link");
    let parent_store = Store::at(linked_dir.join("missing"));
    let mut parent_document = parent_store.load().expect("missing in linked directory");
    assert!(matches!(
        parent_store.save_appearance(&mut parent_document, Appearance::default()),
        Err(Error::Symlink { .. })
    ));
    assert!(!dir.path().join("missing").exists());
}
