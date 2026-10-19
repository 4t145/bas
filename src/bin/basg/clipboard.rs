//! Normalized desktop clipboard contents, independent of the conversation UI.

use gpui_kit::{ClipboardEntry, ClipboardItem, Image};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
pub(crate) mod wayland;

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// # Errors
/// Reports native clipboard negotiation and content read failures.
pub(crate) fn read_native() -> Result<Contents, String> {
    #[cfg(target_os = "linux")]
    {
        wayland::read().map_err(|error| error.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        macos::read().map_err(|error| error.to_string())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Contents {
    pub text: Option<String>,
    pub images: Vec<Image>,
    pub paths: Vec<PathBuf>,
}

impl Contents {
    pub fn from_item(item: Option<ClipboardItem>) -> Self {
        let Some(item) = item else {
            return Self::default();
        };
        let mut contents = Self {
            text: item.text().map(|text| text.replace("\r\n", "\n")),
            ..Self::default()
        };
        for entry in item.entries {
            match entry {
                ClipboardEntry::Image(image) => contents.images.push(image),
                ClipboardEntry::ExternalPaths(paths) => {
                    for path in paths.paths() {
                        if !contents.paths.contains(path) {
                            contents.paths.push(path.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        contents
    }

    pub fn has_attachments(&self) -> bool {
        !self.paths.is_empty() || !self.images.is_empty()
    }
}
