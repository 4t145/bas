//! Same-user Unix sockets. The persistent lockfile serializes crash recovery.

use std::{
    fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions, TryLockError},
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use nix::{libc, unistd::geteuid};
use tokio::net::UnixListener;

use crate::service::Target;

pub(super) type Stream = tokio::net::UnixStream;

const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// Select the per-user local endpoint.
///
/// # Errors
/// Returns an error if the temporary directory cannot be resolved.
pub(super) fn default_target() -> io::Result<Target> {
    let base = match dirs::runtime_dir() {
        Some(path) => path,
        None => fs::canonicalize(std::env::temp_dir())?,
    };
    Ok(Target::Unix(
        base.join(format!("bas-{}", geteuid())).join("service.sock"),
    ))
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn socket_path(target: &Target) -> io::Result<PathBuf> {
    let Target::Unix(path) = target else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unix transport requires a Unix target",
        ));
    };
    let path = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()?.join(path)
    };
    if path.file_name().is_none() || path.components().any(|part| part == Component::ParentDir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path must name a file and contain no parent traversal",
        ));
    }
    Ok(path)
}

// Ancestors must not be replaceable by another user. Sticky directories (e.g.
// /tmp) protect our owned child; symlinks are deliberately not followed.
fn validate_ancestors(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        // The namespace root is the trust anchor. Its owner can be unmapped
        // inside a user namespace; that exception must not apply to descendants.
        if !metadata.is_dir()
            || (ancestor.parent().is_some()
                && metadata.uid() != 0
                && metadata.uid() != geteuid().as_raw())
            || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
        {
            return Err(denied("unsafe socket directory ancestor"));
        }
    }
    Ok(())
}

fn private_parent(path: &Path, create: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing socket parent"))?;
    if let Some(ancestor) = parent.parent() {
        validate_ancestors(ancestor)?;
    }
    if create {
        match DirBuilder::new().mode(DIRECTORY_MODE).create(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o7777 != DIRECTORY_MODE
    {
        return Err(denied(
            "socket parent must be an owned directory with mode 0700",
        ));
    }
    Ok(())
}

fn validate_socket(metadata: &Metadata) -> io::Result<()> {
    if !metadata.file_type().is_socket()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o7777 != FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(denied("socket must be owned by this user with mode 0600"));
    }
    Ok(())
}

fn authenticate(stream: &Stream) -> io::Result<()> {
    if stream.peer_cred()?.uid() != geteuid().as_raw() {
        return Err(denied("Unix peer belongs to a different user"));
    }
    Ok(())
}

/// Connect only to a private socket served by the current effective user.
///
/// # Errors
/// Rejects unsafe paths, non-Unix targets, foreign peers, and connection errors.
pub(super) async fn connect(target: &Target) -> io::Result<Stream> {
    let path = socket_path(target)?;
    private_parent(&path, false)?;
    validate_socket(&fs::symlink_metadata(&path)?)?;
    let stream = Stream::connect(&path).await?;
    authenticate(&stream)?;
    Ok(stream)
}

#[derive(Clone, Copy)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn matches(self, metadata: &Metadata) -> bool {
        self.device == metadata.dev() && self.inode == metadata.ino()
    }
}

fn lock_endpoint(path: &Path) -> io::Result<File> {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&lock_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o7777 != FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(denied("unsafe socket lockfile"));
    }
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => {
            io::Error::new(io::ErrorKind::AddrInUse, "socket listener already running")
        }
        TryLockError::Error(error) => error,
    })?;
    if !Identity::of(&metadata).matches(&fs::symlink_metadata(lock_path)?) {
        return Err(denied("socket lockfile was replaced"));
    }
    // Never unlink this file: another process may already be waiting on its inode.
    Ok(file)
}

struct SocketGuard {
    path: PathBuf,
    identity: Identity,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && self.identity.matches(&metadata)
            && metadata.file_type().is_socket()
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) struct Listener {
    listener: UnixListener,
    target: Target,
    // Field order keeps the endpoint locked until socket cleanup has completed.
    _socket: SocketGuard,
    _lock: File,
}

impl Listener {
    /// Bind a private endpoint, recovering only a verified stale socket.
    ///
    /// # Errors
    /// Rejects unsafe paths, occupied endpoints, non-Unix targets and I/O failures.
    pub(super) async fn bind(target: &Target) -> io::Result<Self> {
        let path = socket_path(target)?;
        private_parent(&path, true)?;
        let lock = lock_endpoint(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_socket(&metadata)?;
                match Stream::connect(&path).await {
                    Ok(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "socket already has a listener",
                        ));
                    }
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
                    Err(error) => return Err(error),
                }
                if !Identity::of(&metadata).matches(&fs::symlink_metadata(&path)?) {
                    return Err(denied("stale socket was replaced"));
                }
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let socket = SocketGuard {
            identity: Identity::of(&fs::symlink_metadata(&path)?),
            path,
        };
        fs::set_permissions(&socket.path, Permissions::from_mode(FILE_MODE))?;
        validate_socket(&fs::symlink_metadata(&socket.path)?)?;
        Ok(Self {
            listener,
            target: target.clone(),
            _socket: socket,
            _lock: lock,
        })
    }

    /// Accept an authenticated stream, discarding foreign or unverifiable peers.
    ///
    /// # Errors
    /// Returns listener I/O errors; peer authentication errors do not stop service.
    pub(super) async fn accept(&mut self) -> io::Result<Stream> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            if authenticate(&stream).is_ok() {
                return Ok(stream);
            }
        }
    }

    pub(super) fn target(&self) -> &Target {
        &self.target
    }
}

#[cfg(test)]
mod tests;
