use std::{io, path::Path};

use super::Error;

pub(super) fn prepare(path: &Path) -> Result<(), Error> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(Error::Invalid("cache requires a parent directory"))?;
    // Inspect ancestors before creating/opening anything; do not silently follow
    // an existing directory symlink, including outside the private leaf.
    for ancestor in parent.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match std::fs::symlink_metadata(ancestor) {
            Ok(info) if !info.is_dir() || info.file_type().is_symlink() => return Err(private()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::symlink_metadata(parent)?.permissions().mode() & 0o077 != 0 {
            return Err(private());
        }
    }
    check_file(path)?;
    // SQLite opens these itself; reject preexisting nonprivate files/symlinks.
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        check_file(Path::new(&name))?;
    }
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    open.open(path)?;
    Ok(())
}

fn check_file(path: &Path) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Ok(info) => {
            if !info.is_file() || info.file_type().is_symlink() {
                return Err(private());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if info.permissions().mode() & 0o077 != 0 || info.nlink() != 1 {
                    return Err(private());
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn private() -> Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "cache paths must be private, regular and not symlinks",
    )
    .into()
}
