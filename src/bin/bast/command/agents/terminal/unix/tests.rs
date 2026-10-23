use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn input(bytes: &[u8]) -> std::os::fd::OwnedFd {
    let (read, write) = nix::unistd::pipe().expect("pipe");
    nix::unistd::write(&write, bytes).expect("pipe input");
    read
}

#[test]
fn utf8_crlf_empty_line_and_no_read_ahead_are_preserved() {
    let cancelled = AtomicBool::new(false);
    let read = input("你好🙂\r\n\nnext\n".as_bytes());
    assert_eq!(super::read(&read, &cancelled).expect("UTF-8"), "你好🙂");
    assert_eq!(super::read(&read, &cancelled).expect("empty line"), "");
    assert_eq!(super::read(&read, &cancelled).expect("next line"), "next");
    assert_eq!(
        super::read(&read, &cancelled).expect_err("EOF").kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn invalid_utf8_and_unterminated_eof_keep_unix_error_semantics() {
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        super::read(&input(b"\xff\n"), &cancelled)
            .expect_err("invalid UTF-8")
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        super::read(&input(b"partial"), &cancelled)
            .expect_err("Unix requires a complete line")
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn cancelled_poll_exits_without_consuming_subsequent_input() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let (read, write) = nix::unistd::pipe().expect("pipe");
    let worker = std::thread::spawn(move || {
        let result = super::read(&read, &worker_cancelled);
        assert_eq!(
            result.expect_err("cancelled").kind(),
            io::ErrorKind::Interrupted
        );
        read
    });
    cancelled.store(true, Ordering::Release);
    let read = worker.join().expect("reader exits without new input");
    nix::unistd::write(&write, b"new answer\n").expect("next input");
    assert_eq!(
        super::read(&read, &AtomicBool::new(false)).expect("next answer"),
        "new answer"
    );
}
