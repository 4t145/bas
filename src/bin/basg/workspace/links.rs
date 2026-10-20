//! One explicit-click policy for Markdown links and ACP resource cards.

use super::{Workspace, navigation};
use gpui_kit::App;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Default)]
pub(super) struct Context {
    directory: Option<PathBuf>,
    local_files: bool,
}

#[derive(Debug)]
pub(super) enum Target {
    External(Url),
    File { uri: Url, path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("{}", crate::i18n::tr!("resource-open-unavailable"))]
    Unavailable,
    #[error("{}", crate::i18n::tr!("resource-invalid-address"))]
    InvalidAddress,
    #[error("{}", crate::i18n::tr!("resource-no-directory"))]
    MissingDirectory,
}

impl Context {
    pub(super) fn new(directory: Option<PathBuf>, local_files: bool) -> Self {
        Self {
            directory,
            local_files,
        }
    }

    /// # Errors
    /// Rejects unsupported schemes, remote filesystem references and relative paths without a base.
    pub(super) fn target(&self, address: &str) -> Result<Target, Error> {
        if address.is_empty() || address.starts_with(['#', '?']) {
            return Err(Error::Unavailable);
        }
        #[cfg(target_os = "windows")]
        if Path::new(address).is_absolute() {
            return self.file(Url::from_file_path(address).map_err(|_| Error::InvalidAddress)?);
        }
        match Url::parse(address) {
            Ok(uri) if matches!(uri.scheme(), "http" | "https" | "mailto") => {
                Ok(Target::External(uri))
            }
            Ok(uri) if uri.scheme() == "file" => self.file(uri),
            Ok(_) => Err(Error::Unavailable),
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                if !self.local_files {
                    return Err(Error::Unavailable);
                }
                let base = if Path::new(address).is_absolute() {
                    Path::new("/")
                } else {
                    self.directory.as_deref().ok_or(Error::MissingDirectory)?
                };
                let uri = Url::from_directory_path(base)
                    .map_err(|_| Error::InvalidAddress)?
                    .join(address)
                    .map_err(|_| Error::InvalidAddress)?;
                self.file(uri)
            }
            Err(_) => Err(Error::InvalidAddress),
        }
    }

    /// # Errors
    /// Requires a local backend and a file URI understood by this platform.
    fn file(&self, uri: Url) -> Result<Target, Error> {
        if !self.local_files {
            return Err(Error::Unavailable);
        }
        let path = uri.to_file_path().map_err(|_| Error::InvalidAddress)?;
        Ok(Target::File { uri, path })
    }

    pub(super) fn open(&self, address: &str, cx: &mut App) {
        match self.target(address) {
            Ok(target) => target.open(cx),
            Err(error) => show_error(error.to_string(), cx),
        }
    }
}

impl Target {
    pub(super) fn open(&self, cx: &mut App) {
        let uri = match self {
            Self::External(uri) => uri,
            Self::File { uri, path } => {
                let error = match path.try_exists() {
                    Ok(true) => None,
                    Ok(false) => Some(crate::i18n::tr!(
                        "resource-file-missing",
                        path = path.display().to_string()
                    )),
                    Err(error) => Some(error.to_string()),
                };
                if let Some(error) = error {
                    show_error(error, cx);
                    return;
                }
                uri
            }
        };
        cx.open_url(uri.as_str());
    }
}

fn show_error(message: String, cx: &mut App) {
    crate::diagnostics::notify(
        None,
        crate::diagnostics::log::Level::Error,
        crate::i18n::tr!("resource-open-error"),
        &message,
        cx,
    );
}

impl Workspace {
    pub(super) fn link_context(&self) -> Context {
        Context::new(
            self.current()
                .and_then(|session| session.cwd.as_deref().map(navigation::groups::normalize))
                .or_else(|| self.navigation.selected_directory.clone()),
            self.backend.server.is_local() || self.backend.server.is_loopback_tcp(),
        )
    }
}

#[cfg(test)]
mod tests;
