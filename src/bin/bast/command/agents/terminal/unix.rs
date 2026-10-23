use std::io;
use std::os::fd::AsFd;
use std::sync::atomic::AtomicBool;

use nix::poll::{PollFd, PollFlags};

const INPUT_POLL_MS: u16 = 100;

/// # Errors
/// Reports cancellation, EOF, invalid UTF-8 or a failed terminal read.
pub(super) fn line(cancelled: &AtomicBool) -> io::Result<String> {
    let stdin = std::io::stdin();
    read(&stdin, cancelled)
}

/// # Errors
/// Reports cancellation, EOF, invalid UTF-8 or an input descriptor error.
fn read(input: &impl AsFd, cancelled: &AtomicBool) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        super::check_cancelled(cancelled)?;
        let mut descriptors = [PollFd::new(input.as_fd(), PollFlags::POLLIN)];
        if nix::poll::poll(&mut descriptors, INPUT_POLL_MS)? == 0 {
            continue;
        }
        super::check_cancelled(cancelled)?;
        let mut byte = [0];
        if nix::unistd::read(input, &mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Terminal input closed",
            ));
        }
        if byte[0] == b'\n' {
            return String::from_utf8(bytes)
                .map(|line| line.trim_end_matches('\r').to_owned())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
        bytes.push(byte[0]);
    }
}

#[cfg(test)]
#[path = "unix/tests.rs"]
mod tests;
