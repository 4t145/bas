use super::*;
use std::os::unix::{ffi::OsStringExt, fs::symlink};

#[test]
fn canonicalization_rejects_non_utf8_even_through_utf8_symlinks() {
    let root = tempfile::tempdir().expect("temporary directory is required for path fixtures");
    let invalid = root
        .path()
        .join(std::ffi::OsString::from_vec(b"cwd-\xff".to_vec()));
    std::fs::create_dir(&invalid).expect("non-UTF-8 directory fixture must exist");
    let link = root.path().join("valid-utf8-link");
    symlink(&invalid, &link).expect("UTF-8 symlink must point to the non-UTF-8 fixture");
    for path in [&invalid, &link] {
        let error = protocol_cwd(path).expect_err("canonical non-UTF-8 paths cannot cross RPC");
        assert!(matches!(
            error.downcast_ref::<basr::service::Error>(),
            Some(basr::service::Error::NonUtf8Cwd)
        ));
    }
    let valid = root.path().join("中文");
    std::fs::create_dir(&valid).expect("UTF-8 directory fixture must exist");
    assert_eq!(
        protocol_cwd(&valid).expect("existing UTF-8 directory is valid for RPC"),
        valid.to_str().expect("fixture path contains only UTF-8")
    );
}
