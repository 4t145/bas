//! Build identity without timestamps, host paths, or repository URLs.

use std::{env, path::Path, process::Command};

#[cfg(test)]
#[path = "build_metadata/tests.rs"]
mod tests;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn revision_tag(revision: Option<&str>) -> String {
    match revision {
        Some(revision)
            if matches!(revision.len(), 40 | 64)
                && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            // HEAD identifies the base, not necessarily the compiled working tree.
            format!("git-{}-worktree", &revision[..12])
        }
        _ => "source-unknown".into(),
    }
}

fn validate_override(value: &str) -> Result<&str, &'static str> {
    if !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        Ok(value)
    } else {
        Err("BAS_BUILD_TAG must contain 1–80 ASCII letters, digits, '.', '_', '-', or '+'")
    }
}

/// Emits Cargo's build tag and the metadata dependencies used to derive it.
///
/// # Errors
/// Returns an error for an invalid explicit override or missing Cargo environment.
pub fn emit() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build_metadata.rs");
    println!("cargo:rerun-if-env-changed=BAS_BUILD_TAG");
    let root = std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let tag = match env::var("BAS_BUILD_TAG") {
        Ok(value) => validate_override(&value)?.to_owned(),
        Err(env::VarError::NotUnicode(_)) => return Err("BAS_BUILD_TAG must be ASCII".into()),
        Err(env::VarError::NotPresent) => {
            // Do not accidentally identify an enclosing repository for an unpacked archive.
            println!("cargo:rerun-if-changed=.git");
            let revision = if root.join(".git").exists() {
                for entry in ["HEAD", "refs", "packed-refs"] {
                    if let Some(path) = git(&root, &["rev-parse", "--git-path", entry]) {
                        println!("cargo:rerun-if-changed={}", root.join(path).display());
                    }
                }
                git(&root, &["rev-parse", "--verify", "HEAD"])
            } else {
                None
            };
            revision_tag(revision.as_deref())
        }
    };
    println!("cargo:rustc-env=BAS_BUILD_TAG={tag}");
    Ok(())
}
