//! Best-effort, bounded storage for validated remote artwork.
use std::{
    fs::{self, File, Metadata, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::MAX_ICON_BYTES;

const MAX_RECORD: u64 = 3 * 1024 * 1024;
const MAX_MIME: usize = 256;
const MAX_VALIDATOR: usize = 8 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const LOCK_TIMEOUT: Duration = Duration::from_millis(250);
const LOCK_RETRY: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub fetched_at: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("icon cache IO error")]
    Io(#[from] io::Error),
    #[error("invalid icon cache record")]
    Invalid,
    #[error("icon cache requires private regular paths")]
    UnsafePath,
}

#[derive(Clone, Debug)]
pub struct Cache {
    directory: PathBuf,
    max_total: u64,
    max_entries: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    bytes: String,
    mime: String,
    fetched_at: u64,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl Cache {
    pub fn discover() -> Option<Self> {
        // Non-Unix persistence needs an equivalent private-ACL implementation.
        if cfg!(unix) {
            dirs::cache_dir().map(|path| Self::at(path.join("basg/agent-icons-v1")))
        } else {
            None
        }
    }

    pub fn at(directory: PathBuf) -> Self {
        Self {
            directory,
            max_total: MAX_TOTAL,
            max_entries: MAX_ENTRIES,
        }
    }

    /// # Errors
    /// Rejects unsafe paths, malformed/oversized records and filesystem failures.
    pub fn read(&self, key: &str) -> Result<Option<Entry>, Error> {
        let _lock = self.lock()?;
        let path = self.path(key);
        let Some(info) = check_file(&path)? else {
            return Ok(None);
        };
        if info.len() > MAX_RECORD {
            return Err(Error::Invalid);
        }
        let mut file = open_file(&path, false)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_RECORD + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err(Error::Invalid);
        }
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if record.version != 1 {
            return Err(Error::Invalid);
        }
        let entry = Entry {
            bytes: STANDARD.decode(record.bytes).map_err(|_| Error::Invalid)?,
            mime: record.mime,
            fetched_at: record.fetched_at,
            etag: record.etag,
            last_modified: record.last_modified,
        };
        validate(&entry)?;
        // Access time is independent of HTTP freshness and need not be precise.
        let _ = file.set_modified(SystemTime::now());
        Ok(Some(entry))
    }

    /// # Errors
    /// Rejects oversized entries, unsafe paths and filesystem failures.
    pub fn write(&self, key: &str, entry: &Entry) -> Result<(), Error> {
        validate(entry)?;
        let record = Record {
            version: 1,
            bytes: STANDARD.encode(&entry.bytes),
            mime: entry.mime.clone(),
            fetched_at: entry.fetched_at,
            etag: entry.etag.clone(),
            last_modified: entry.last_modified.clone(),
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| Error::Invalid)?;
        if bytes.len() as u64 > MAX_RECORD
            || bytes.len() as u64 > self.max_total
            || self.max_entries == 0
        {
            return Err(Error::Invalid);
        }
        let _lock = self.lock()?;
        let path = self.path(key);
        check_file(&path)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|error| error.error)?;
        self.evict(&path)
    }

    /// # Errors
    /// Rejects unsafe paths and filesystem failures. Missing entries are harmless.
    pub fn remove(&self, key: &str) -> Result<(), Error> {
        let _lock = self.lock()?;
        let path = self.path(key);
        if check_file(&path)?.is_some() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn path(&self, key: &str) -> PathBuf {
        self.directory
            .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
    }

    fn lock(&self) -> Result<File, Error> {
        // As with the connection cache, never follow existing directory symlinks
        // or change permissions on directories that belong to the user.
        for path in self.directory.ancestors() {
            if path.as_os_str().is_empty() {
                continue;
            }
            match fs::symlink_metadata(path) {
                Ok(info) if !info.is_dir() => return Err(Error::UnsafePath),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.directory)?;
        private(&fs::symlink_metadata(&self.directory)?)?;
        let path = self.directory.join(".lock");
        check_file(&path)?;
        let file = open_file(&path, true)?;
        // Serialize accounting and replacement across processes, not just handles.
        // A frozen peer must not prevent loading artwork from the network.
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(LOCK_RETRY);
                }
                Err(error) => return Err(io::Error::from(error).into()),
            }
        }
        Ok(file)
    }

    fn evict(&self, keep: &Path) -> Result<(), Error> {
        let mut entries = Vec::new();
        let mut total = 0;
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            if !owned_name(&item.file_name().to_string_lossy()) {
                continue;
            }
            let path = item.path();
            // Foreign/insecure leaves are never read, counted, or removed.
            let info = match check_file(&path) {
                Ok(Some(info)) => info,
                Ok(None) | Err(Error::UnsafePath) => continue,
                Err(error) => return Err(error),
            };
            total += info.len();
            entries.push((info.modified()?, path, info.len()));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut count = entries.len();
        for (_, path, size) in entries {
            if total <= self.max_total && count <= self.max_entries {
                break;
            }
            if path == keep {
                continue;
            }
            fs::remove_file(path)?;
            total -= size;
            count -= 1;
        }
        Ok(())
    }
}

fn validate(entry: &Entry) -> Result<(), Error> {
    if entry.bytes.len() as u64 > MAX_ICON_BYTES
        || entry.mime.is_empty()
        || entry.mime.len() > MAX_MIME
        || entry.mime.chars().any(char::is_control)
        || [&entry.etag, &entry.last_modified]
            .into_iter()
            .flatten()
            .any(|value| value.len() > MAX_VALIDATOR || value.chars().any(char::is_control))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn owned_name(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn private(info: &Metadata) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if info.permissions().mode() & 0o077 != 0
            || info.uid() != nix::unistd::geteuid().as_raw()
            || (info.is_file() && info.nlink() != 1)
        {
            return Err(Error::UnsafePath);
        }
    }
    #[cfg(not(unix))]
    let _ = info;
    Ok(())
}

fn check_file(path: &Path) -> Result<Option<Metadata>, Error> {
    match fs::symlink_metadata(path) {
        Ok(info) => {
            if !info.is_file() {
                return Err(Error::UnsafePath);
            }
            private(&info)?;
            Ok(Some(info))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn open_file(path: &Path, create: bool) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let info = file.metadata()?;
    if !info.is_file() {
        return Err(Error::UnsafePath);
    }
    private(&info)?;
    Ok(file)
}

#[cfg(test)]
mod tests;
