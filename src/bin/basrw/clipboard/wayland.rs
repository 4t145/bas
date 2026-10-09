//! Uses the published data-control client without overriding GPUI's platform crate.

use super::Contents;
use gpui_kit::{Image, ImageFormat};
use nix::poll::{PollFd, PollFlags, poll};
use std::{
    io::{self, Read},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use strum::IntoEnumIterator;
use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_BYTES: usize = 16 * 1024 * 1024;
const TEXT_TYPES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "UTF8_STRING",
    "text/plain",
];

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Wayland clipboard is unavailable: {0}")]
    Clipboard(#[from] paste::Error),
    #[error("cannot read clipboard: {0}")]
    Io(#[from] io::Error),
    #[error("clipboard contents exceed 16 MiB")]
    TooLarge,
    #[error("clipboard contains invalid UTF-8")]
    InvalidText,
}

/// # Errors
/// Reports unavailable data-control, invalid text, stalled sources or oversized data.
pub(crate) fn read() -> Result<Contents, Error> {
    let types = match paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
        Ok(types) => types,
        Err(paste::Error::ClipboardEmpty | paste::Error::NoMimeType | paste::Error::NoSeats) => {
            return Ok(Contents::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut contents = Contents::default();
    if types.contains("text/uri-list") {
        let data = read_mime("text/uri-list")?;
        let text = std::str::from_utf8(&data).map_err(|_| Error::InvalidText)?;
        contents.paths = file_paths(text);
    }
    if let Some(mime) = TEXT_TYPES.into_iter().find(|mime| types.contains(*mime)) {
        let data = read_mime(mime)?;
        contents.text = Some(
            String::from_utf8(data)
                .map_err(|_| Error::InvalidText)?
                .replace("\r\n", "\n"),
        );
    }
    if contents.paths.is_empty() {
        for format in ImageFormat::iter() {
            if types.contains(format.mime_type()) {
                contents
                    .images
                    .push(Image::from_bytes(format, read_mime(format.mime_type())?));
                break;
            }
        }
    }
    Ok(contents)
}

fn file_paths(text: &str) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let Ok(uri) = url::Url::parse(line) else {
            continue;
        };
        // File copying never downloads URLs or opens a remote host path.
        if uri.scheme() != "file" || uri.host_str().is_some_and(|host| host != "localhost") {
            continue;
        }
        if let Ok(path) = uri.to_file_path()
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

fn read_mime(mime: &str) -> Result<Vec<u8>, Error> {
    let (mut pipe, _) = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(mime),
    )?;
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = remaining.as_millis().min(u16::MAX as u128) as u16;
        if timeout == 0
            || poll(&mut [PollFd::new(pipe.as_fd(), PollFlags::POLLIN)], timeout)
                .map_err(io::Error::from)?
                == 0
        {
            return Err(
                io::Error::new(io::ErrorKind::TimedOut, "clipboard source timed out").into(),
            );
        }
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "gui-tests")]
mod probe;
#[cfg(feature = "gui-tests")]
pub(crate) use probe::check;
