use super::*;

#[test]
fn archive_checksum_and_traversal_are_rejected() {
    assert!(verify_digest(&"a".repeat(64), Some(&"b".repeat(64))).is_err());
    assert!(verify_digest(&"a".repeat(64), Some(&"A".repeat(64))).is_ok());
    let directory = tempfile::tempdir().expect("temporary directory");
    let archive_path = directory.path().join("agent.tgz");
    let gzip = flate2::write::GzEncoder::new(
        File::create(&archive_path).expect("archive"),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_size(6);
    header.set_mode(0o755);
    header.as_mut_bytes()[..10].copy_from_slice(b"../escaped");
    header.set_cksum();
    builder
        .append(&header, &b"unsafe"[..])
        .expect("malicious archive fixture");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip");
    let destination = directory.path().join("extracted");
    fs::create_dir(&destination).expect("extract directory");
    assert!(
        extract(
            &archive_path,
            "https://example.com/agent.tgz",
            &destination,
            "agent",
            false
        )
        .is_err()
    );
    assert!(!directory.path().join("escaped").exists());
}

#[test]
fn archives_cannot_create_symlinks_that_escape_the_root() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let archive_path = directory.path().join("agent.tgz");
    let gzip = flate2::write::GzEncoder::new(
        File::create(&archive_path).expect("archive"),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_path("escape").expect("relative link");
    header.set_size(0);
    header.set_mode(0o777);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name("../../outside").expect("link name");
    header.set_cksum();
    builder.append(&header, io::empty()).expect("append link");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip");
    let destination = directory.path().join("extracted");
    fs::create_dir(&destination).expect("extract directory");
    assert!(
        extract(
            &archive_path,
            "https://example.com/agent.tgz",
            &destination,
            "agent",
            false
        )
        .is_err()
    );
    assert!(!destination.join("escape").exists());
}
