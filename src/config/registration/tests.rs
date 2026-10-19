use super::*;
use crate::config::agent::Command;
use serde_json::json;

fn fixture(id: &str, version: u32, spec: &AgentSpec) -> (tempfile::TempDir, ManagedAgent) {
    let dir = tempfile::tempdir().expect("installation directory");
    let path = dir
        .path()
        .canonicalize()
        .expect("absolute directory")
        .join(INSTALLATION_FILE);
    let bytes = serde_json::to_vec(&json!({
        "schema_version": version, "spec": spec, "daemon_metadata": {"version": "1"}
    }))
    .expect("manifest bytes");
    let installation =
        InstallationRef::from_bytes(path.clone(), &bytes).expect("reference before publish");
    fs::write(path, bytes).expect("publish manifest");
    (
        dir,
        ManagedAgent {
            id: id.into(),
            installation,
        },
    )
}

#[test]
fn validates_shapes_without_reading_files() {
    let dir = tempfile::tempdir().expect("directory");
    for path in [
        PathBuf::from(INSTALLATION_FILE),
        dir.path().join("..").join(INSTALLATION_FILE),
        dir.path().join("wrong.json"),
    ] {
        assert!(InstallationRef::from_bytes(path, b"").is_err());
    }
    let mut managed = ManagedAgent {
        id: "a".into(),
        installation: InstallationRef::from_bytes(dir.path().join(INSTALLATION_FILE), b"")
            .expect("unpublished"),
    };
    let registration = AgentRegistration::Managed(managed.clone());
    validate_registrations(std::slice::from_ref(&registration)).expect("no filesystem access");
    assert!(registration.resolve().is_err());
    assert!(validate_registrations(&[]).is_err());
    assert!(validate_registrations(&[registration.clone(), registration]).is_err());
    managed.installation.sha256 = "invalid".into();
    assert!(validate_registrations(&[AgentRegistration::Managed(managed.clone())]).is_err());
    managed.id = " ".into();
    assert!(validate_registrations(&[AgentRegistration::Managed(managed)]).is_err());
    let mut invalid = AgentSpec::embedded();
    invalid.cancellation_grace = std::time::Duration::ZERO;
    assert!(validate_registrations(&[invalid.into()]).is_err());
}

#[test]
fn changed_corrupt_oversized_and_nonfile_manifests_fail_closed() {
    let (_dir, mut managed) = fixture("a", 1, &AgentSpec::acp("a", Command::new("a", [])));
    fs::write(&managed.installation.path, b"{}").expect("changed bytes");
    assert!(matches!(
        managed.resolve(),
        Err(RegistrationError::Reference { .. })
    ));
    managed.installation = InstallationRef::from_bytes(managed.installation.path.clone(), b"{")
        .expect("corrupt reference");
    fs::write(&managed.installation.path, b"{").expect("corrupt bytes");
    assert!(matches!(managed.resolve(), Err(RegistrationError::Json(_))));
    let oversized = vec![0; MAX_MANIFEST_BYTES as usize + 1];
    assert!(InstallationRef::from_bytes(managed.installation.path.clone(), &oversized).is_err());
    fs::write(&managed.installation.path, oversized).expect("oversized bytes");
    assert!(managed.resolve().is_err());
    fs::remove_file(&managed.installation.path).expect("remove manifest");
    assert!(managed.resolve().is_err());
    fs::create_dir(&managed.installation.path).expect("not a regular file");
    assert!(managed.resolve().is_err());
}

#[cfg(unix)]
#[test]
fn symlink_files_and_ancestors_are_rejected() {
    use std::os::unix::fs::symlink;
    let (dir, managed) = fixture("a", 1, &AgentSpec::acp("a", Command::new("a", [])));
    let target = dir.path().join("original");
    fs::rename(&managed.installation.path, &target).expect("move original");
    symlink(&target, &managed.installation.path).expect("manifest link");
    assert!(managed.resolve().is_err());
    fs::remove_file(&managed.installation.path).expect("remove link");
    fs::rename(&target, &managed.installation.path).expect("restore original");
    let link_dir = tempfile::tempdir().expect("link parent");
    let link = link_dir.path().join("linked");
    symlink(dir.path(), &link).expect("ancestor link");
    let linked = InstallationRef {
        path: link.join(INSTALLATION_FILE),
        ..managed.installation
    };
    assert!(linked.read().is_err());
}
