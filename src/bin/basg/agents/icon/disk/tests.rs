use super::*;

fn fixture() -> (tempfile::TempDir, Cache) {
    let root = tempfile::tempdir().expect("temporary directory");
    let cache = Cache::at(root.path().join("icons"));
    (root, cache)
}

fn entry(value: u8) -> Entry {
    Entry {
        bytes: vec![value; 32],
        mime: "image/png".into(),
        fetched_at: 1234,
        etag: Some("\"version\"".into()),
        last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
    }
}

#[test]
fn reopen_preserves_metadata_and_full_keys_are_isolated() {
    let (_root, cache) = fixture();
    let keys = [
        "https://one.example/icon.png?secret=one",
        "https://one.example/icon.png?secret=two",
        "https://two.example/icon.png?secret=one",
    ];
    assert!(!cache.directory.exists());
    for (index, key) in keys.iter().enumerate() {
        cache.write(key, &entry(index as u8)).expect("write");
        let record = fs::read_to_string(cache.path(key)).expect("record");
        assert!(!record.contains(key));
        assert!(!record.contains("secret"));
    }
    let reopened = Cache::at(cache.directory);
    for (index, key) in keys.iter().enumerate() {
        assert_eq!(reopened.read(key).expect("read"), Some(entry(index as u8)));
    }
    reopened.remove(keys[0]).expect("remove");
    reopened.remove(keys[0]).expect("missing remove");
    assert_eq!(reopened.read(keys[0]).expect("missing"), None);
}

#[test]
fn corrupt_records_are_bounded_and_do_not_affect_other_entries() {
    let (_root, cache) = fixture();
    cache.write("good", &entry(1)).expect("write");
    cache.write("bad", &entry(2)).expect("write");
    let original = fs::read(cache.path("bad")).expect("record");
    let mut invalid_base64: serde_json::Value = serde_json::from_slice(&original).expect("json");
    invalid_base64["bytes"] = "!!".into();
    let mut invalid_version = invalid_base64.clone();
    invalid_version["version"] = 2.into();
    let mut invalid_metadata: serde_json::Value = serde_json::from_slice(&original).expect("json");
    invalid_metadata["mime"] = "a".repeat(MAX_MIME + 1).into();
    let mut excessive_raw = invalid_base64.clone();
    excessive_raw["bytes"] = STANDARD.encode(vec![0; MAX_ICON_BYTES as usize + 1]).into();
    for bytes in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        original[..original.len() / 2].to_vec(),
        serde_json::to_vec(&invalid_base64).expect("json"),
        serde_json::to_vec(&invalid_version).expect("json"),
        serde_json::to_vec(&invalid_metadata).expect("json"),
        serde_json::to_vec(&excessive_raw).expect("json"),
        vec![b' '; MAX_RECORD as usize + 1],
    ] {
        fs::write(cache.path("bad"), bytes).expect("corrupt existing private file");
        assert!(matches!(cache.read("bad"), Err(Error::Invalid)));
        assert_eq!(cache.read("good").expect("good"), Some(entry(1)));
    }
    let mut excessive = entry(1);
    excessive.bytes = vec![0; MAX_ICON_BYTES as usize + 1];
    assert!(cache.write("bad", &excessive).is_err());
    excessive = entry(1);
    excessive.etag = Some("a".repeat(MAX_VALIDATOR + 1));
    assert!(cache.write("bad", &excessive).is_err());
    excessive = entry(1);
    excessive.mime = "x\r\nx".into();
    assert!(cache.write("bad", &excessive).is_err());
}

#[test]
fn reading_updates_access_order_without_changing_freshness() {
    let (_root, mut cache) = fixture();
    cache.max_entries = 2;
    cache.write("first", &entry(1)).expect("first");
    cache.write("second", &entry(2)).expect("second");
    for key in ["first", "second"] {
        open_file(&cache.path(key), false)
            .expect("open")
            .set_modified(SystemTime::UNIX_EPOCH)
            .expect("age");
    }
    assert_eq!(cache.read("first").expect("touch"), Some(entry(1)));
    assert!(
        fs::metadata(cache.path("first"))
            .expect("metadata")
            .modified()
            .expect("mtime")
            > SystemTime::UNIX_EPOCH
    );
    cache.write("third", &entry(3)).expect("third");
    assert!(cache.read("second").expect("evicted").is_none());
    assert_eq!(cache.read("first").expect("retained"), Some(entry(1)));
}

#[test]
fn concurrent_handles_only_observe_complete_replacements() {
    let (_root, cache) = fixture();
    cache.write("key", &entry(0)).expect("initial write");
    std::thread::scope(|scope| {
        for index in 0..4 {
            let cache = Cache::at(cache.directory.clone());
            scope.spawn(move || {
                for _ in 0..20 {
                    cache.write("key", &entry(index)).expect("replace");
                    let found = cache.read("key").expect("read").expect("entry");
                    assert_eq!(found, entry(found.bytes[0]));
                }
            });
        }
    });
    assert_eq!(fs::read_dir(&cache.directory).expect("list").count(), 2);
}

#[test]
fn quotas_use_record_sizes_and_access_order_only_for_owned_files() {
    let (_root, mut cache) = fixture();
    cache.max_entries = 2;
    cache.write("old", &entry(1)).expect("old");
    let old = open_file(&cache.path("old"), false).expect("open");
    old.set_modified(SystemTime::UNIX_EPOCH).expect("age");
    cache.write("recent", &entry(2)).expect("recent");
    fs::write(cache.directory.join("unrelated.txt"), "leave alone").expect("foreign");
    cache.write("new", &entry(3)).expect("new");
    assert!(cache.read("old").expect("old").is_none());
    assert!(cache.read("recent").expect("recent").is_some());
    let size = fs::metadata(cache.path("new")).expect("size").len();
    cache.max_total = size;
    cache.write("last", &entry(4)).expect("quota write");
    assert!(cache.read("new").expect("evicted").is_none());
    assert!(cache.read("recent").expect("evicted").is_none());
    assert_eq!(cache.read("last").expect("last"), Some(entry(4)));
    assert_eq!(
        fs::read_to_string(cache.directory.join("unrelated.txt")).expect("foreign"),
        "leave alone"
    );
}

#[cfg(unix)]
#[test]
fn unsafe_paths_are_refused_without_chmod_or_following_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (root, cache) = fixture();
    cache.write("key", &entry(1)).expect("write");
    assert_eq!(
        fs::metadata(&cache.directory)
            .expect("dir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(cache.path("key"))
            .expect("file")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let linked = root.path().join("linked");
    symlink(&cache.directory, &linked).expect("symlink");
    assert!(Cache::at(linked).read("key").is_err());
    let target = root.path().join("target");
    fs::write(&target, "untouched").expect("target");
    cache.remove("key").expect("remove");
    symlink(&target, cache.path("key")).expect("file link");
    assert!(cache.read("key").is_err());
    assert!(cache.write("key", &entry(2)).is_err());
    assert!(cache.remove("key").is_err());
    assert_eq!(fs::read_to_string(&target).expect("target"), "untouched");
    fs::remove_file(cache.path("key")).expect("unlink test symlink");
    fs::create_dir(cache.path("key")).expect("nonregular leaf");
    assert!(cache.read("key").is_err());
    fs::remove_dir(cache.path("key")).expect("remove test dir");
    cache.write("key", &entry(1)).expect("write");
    fs::set_permissions(cache.path("key"), fs::Permissions::from_mode(0o644))
        .expect("insecure file");
    assert!(cache.read("key").is_err());
    assert!(cache.remove("key").is_err());
    fs::set_permissions(&cache.directory, fs::Permissions::from_mode(0o755))
        .expect("insecure directory");
    assert!(cache.read("other").is_err());
    assert_eq!(
        fs::metadata(&cache.directory)
            .expect("dir")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}
