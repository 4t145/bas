use super::{Error, Progress, registry};
use anyhow::{Context, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
};
use tokio::io::AsyncWriteExt;

const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EXTRACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXTRACT_FILES: usize = 100_000;

/// # Errors
/// Rejects invalid URLs, cancellation, failed downloads, oversized archives and checksum mismatches.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    checksum: Option<&str>,
    progress: &Progress,
) -> Result<()> {
    let request = client.get(registry::download_url(url)?).send();
    let response = tokio::select! { result = request => result?.error_for_status()?, _ = progress.closed() => return Err(Error::Cancelled.into()) };
    registry::download_url(response.url().as_str())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_DOWNLOAD_BYTES)
    {
        return Err(Error::ArchiveLimit.into());
    }
    let mut file = tokio::fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    loop {
        let chunk = tokio::select! { next = stream.next() => next, _ = progress.closed() => return Err(Error::Cancelled.into()) };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk?;
        total += chunk.len() as u64;
        if total > MAX_DOWNLOAD_BYTES {
            return Err(Error::ArchiveLimit.into());
        }
        digest.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    verify_digest(&format!("{:x}", digest.finalize()), checksum)?;
    Ok(())
}

/// # Errors
/// Rejects a checksum that differs from the downloaded archive.
fn verify_digest(actual: &str, expected: Option<&str>) -> Result<(), Error> {
    if expected.is_some_and(|value| !value.eq_ignore_ascii_case(actual)) {
        return Err(Error::Checksum);
    }
    Ok(())
}

/// # Errors
/// Rejects links, unsafe paths, special files and archives exceeding extraction limits.
pub fn extract(
    path: &Path,
    url: &str,
    destination: &Path,
    command: &str,
    skip_links: bool,
) -> Result<()> {
    let url = registry::download_url(url)?;
    let name = url.path().to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar(
            flate2::read::GzDecoder::new(File::open(path)?),
            destination,
            skip_links,
        )?;
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        extract_tar(
            bzip2::read::BzDecoder::new(File::open(path)?),
            destination,
            skip_links,
        )?;
    } else if name.ends_with(".zip") {
        extract_zip(path, destination)?;
    } else {
        if [
            ".dmg",
            ".pkg",
            ".deb",
            ".rpm",
            ".msi",
            ".appimage",
            ".tar.xz",
        ]
        .iter()
        .any(|extension| name.ends_with(extension))
        {
            return Err(Error::UnsupportedArchive.into());
        }
        let target = destination.join(registry::safe_relative(command)?);
        fs::create_dir_all(target.parent().context("raw binary has no parent")?)?;
        fs::copy(path, target)?;
    }
    Ok(())
}

fn extract_tar(reader: impl Read, destination: &Path, skip_links: bool) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut total = 0_u64;
    for (count, entry) in archive.entries()?.enumerate() {
        if count >= MAX_EXTRACT_FILES {
            return Err(Error::ArchiveLimit.into());
        }
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        // A root directory entry is harmless; no other entry may refer to the root itself.
        if entry.header().entry_type().is_dir() && (name == "." || name == "./") {
            continue;
        }
        let target = destination.join(registry::safe_relative(&name)?);
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            if skip_links {
                continue;
            }
            return Err(Error::ArchiveLink(name).into());
        }
        if kind.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        if !kind.is_file() {
            return Err(Error::ArchiveLink(name).into());
        }
        total = total.checked_add(entry.size()).ok_or(Error::ArchiveLimit)?;
        if total > MAX_EXTRACT_BYTES {
            return Err(Error::ArchiveLimit.into());
        }
        fs::create_dir_all(target.parent().context("archive entry has no parent")?)?;
        let mut file = File::options().write(true).create_new(true).open(&target)?;
        io::copy(&mut entry, &mut file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.header().mode()? & 0o777;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

fn extract_zip(path: &Path, destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    if archive.len() > MAX_EXTRACT_FILES {
        return Err(Error::ArchiveLimit.into());
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() && (entry.name() == "." || entry.name() == "./") {
            continue;
        }
        let target = destination.join(registry::safe_relative(entry.name())?);
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(Error::ArchiveLink(entry.name().into()).into());
        }
        if entry.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        total = total.checked_add(entry.size()).ok_or(Error::ArchiveLimit)?;
        if total > MAX_EXTRACT_BYTES {
            return Err(Error::ArchiveLimit.into());
        }
        fs::create_dir_all(target.parent().context("archive entry has no parent")?)?;
        let mut file = File::options().write(true).create_new(true).open(&target)?;
        let size = entry.size();
        let copied = io::copy(&mut (&mut entry).take(size + 1), &mut file)?;
        if copied != size {
            return Err(Error::ArchiveLimit.into());
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

/// # Errors
/// Rejects missing executables and symbolic links; sets user executable permission on Unix.
pub fn executable(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::MissingExecutable(path.into()).into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )?;
    }
    adapt_managed_elf(path)?;
    Ok(())
}

fn adapt_managed_elf(path: &Path) -> Result<()> {
    let Some(patchelf) = std::env::var_os("BASRD_ACP_PATCHELF") else {
        return Ok(());
    };
    let Some(loader) = std::env::var_os("BASRD_ACP_ELF_LOADER") else {
        return Ok(());
    };
    let Some(libraries) = std::env::var_os("BASRD_ACP_ELF_LIBRARY_PATH") else {
        return Ok(());
    };
    let mut magic = [0_u8; 4];
    if File::open(path)?.read(&mut magic)? != magic.len() || magic != *b"\x7fELF" {
        return Ok(());
    }
    // Shared libraries and static executables have no interpreter to adapt.
    let interpreter = std::process::Command::new(&patchelf)
        .arg("--print-interpreter")
        .arg(path)
        .output()?;
    if !interpreter.status.success() || interpreter.stdout.is_empty() {
        return Ok(());
    }
    let rpath = std::process::Command::new(&patchelf)
        .arg("--print-rpath")
        .arg(path)
        .output()?;
    let mut combined = libraries;
    if rpath.status.success() {
        let existing = String::from_utf8(rpath.stdout)?.trim().to_owned();
        if !existing.is_empty() {
            combined.push(":");
            combined.push(existing);
        }
    }
    let status = std::process::Command::new(patchelf)
        .arg("--set-interpreter")
        .arg(loader)
        .arg("--set-rpath")
        .arg(combined)
        .arg(path)
        .status()?;
    if !status.success() {
        anyhow::bail!("could not adapt managed ELF executable to the daemon's packaged runtime");
    }
    Ok(())
}

/// # Errors
/// Reports failed adaptations of bundled executable ELF files in managed npm packages.
pub fn adapt_tree(directory: &Path) -> Result<()> {
    if std::env::var_os("BASRD_ACP_PATCHELF").is_none() {
        return Ok(());
    }
    let mut directories = vec![directory.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                adapt_managed_elf(&entry.path())?;
            }
        }
    }
    Ok(())
}

/// # Errors
/// Rejects symbolic links in any managed storage directory.
pub fn safe_directory(path: &Path) -> Result<()> {
    reject_directory_links(path)?;
    fs::create_dir_all(path)?;
    Ok(())
}

/// # Errors
/// Rejects symbolic links in any managed storage ancestor without creating directories.
pub fn reject_directory_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::StorageLink(ancestor.into()).into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// # Errors
/// Rejects linked storage files and failed atomic writes.
pub fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("metadata has no directory")?;
    safe_directory(parent)?;
    reject_file_link(path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

/// # Errors
/// Rejects symbolic links and I/O failures while inspecting a metadata file.
pub fn reject_file_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(Error::StorageLink(path.into()).into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests;
