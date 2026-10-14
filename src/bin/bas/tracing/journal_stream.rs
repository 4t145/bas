use std::{env, ffi::OsStr, io, os::fd::AsFd};

use nix::sys::stat::fstat;

#[cfg(test)]
mod tests;

const JOURNAL_STREAM_ENV: &str = "JOURNAL_STREAM";

pub(super) fn stdout_is_journal() -> bool {
    stream_is_journal(env::var_os(JOURNAL_STREAM_ENV).as_deref(), io::stdout())
}

fn stream_is_journal(value: Option<&OsStr>, stream: impl AsFd) -> bool {
    let Some(value) = value.and_then(OsStr::to_str) else {
        return false;
    };
    let Some((device, inode)) = value.split_once(':') else {
        return false;
    };
    let (Ok(device), Ok(inode)) = (device.parse::<u64>(), inode.parse::<u64>()) else {
        return false;
    };
    let Ok(stat) = fstat(stream) else {
        return false;
    };

    // An inherited variable is insufficient after stdout is redirected.
    // https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#%24JOURNAL_STREAM
    u128::from(stat.st_dev) == u128::from(device) && u128::from(stat.st_ino) == u128::from(inode)
}
