//! Keep demo file links valid for the lifetime of their Agent connection.

use std::sync::Mutex;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct Resources {
    directory: Mutex<Option<tempfile::TempDir>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("could not create the sample resource: {0}")]
    Io(#[from] std::io::Error),
    #[error("the sample resource path cannot be represented as a file URI")]
    InvalidUri,
}

impl Resources {
    /// # Errors
    /// Returns an error if the temporary resource cannot be written or addressed.
    pub(crate) fn sample_uri(&self) -> Result<String, Error> {
        let mut directory = self.directory.lock().expect("resource directory mutex");
        if directory.is_none() {
            let temporary = tempfile::Builder::new()
                .prefix("acp-test-agent-")
                .tempdir()?;
            std::fs::write(temporary.path().join("sample.md"), super::SAMPLE_RESOURCE)?;
            *directory = Some(temporary);
        }
        let path = directory
            .as_ref()
            .expect("resource directory initialized")
            .path()
            .join("sample.md");
        url::Url::from_file_path(path)
            .map(String::from)
            .map_err(|_| Error::InvalidUri)
    }
}
