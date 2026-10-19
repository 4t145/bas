use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

use nix::{sys::stat::fstat, unistd::pipe};

use super::stream_is_journal;

#[test]
fn only_the_matching_stream_is_accepted() {
    let (_reader, writer) = pipe().expect("original test pipe should open");
    let (_other_reader, other_writer) = pipe().expect("redirected test pipe should open");
    let stat = fstat(&writer).expect("open pipe should have metadata");
    let value = format!("{}:{}", stat.st_dev, stat.st_ino);

    assert!(stream_is_journal(Some(OsStr::new(&value)), &writer));
    assert!(!stream_is_journal(Some(OsStr::new(&value)), &other_writer));
}

#[test]
fn missing_invalid_and_non_unicode_values_are_rejected() {
    let (_reader, writer) = pipe().expect("test pipe should open");
    assert!(!stream_is_journal(None, &writer));
    assert!(!stream_is_journal(
        Some(OsStr::from_bytes(b"1:\xff")),
        &writer
    ));
    for value in [
        "",
        ":",
        "1",
        "1:",
        ":1",
        "1:2:3",
        "device:inode",
        "-1:2",
        "1:-2",
        " 1:2",
        "1:2 ",
        "1:2\n",
        "18446744073709551616:2",
        "1:18446744073709551616",
    ] {
        assert!(
            !stream_is_journal(Some(OsStr::new(value)), &writer),
            "accepted invalid stream identity {value:?}"
        );
    }
}
