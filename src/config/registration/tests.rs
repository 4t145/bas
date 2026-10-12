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
fn custom_roundtrip_keeps_flat_spec_shape() {
    let spec = AgentSpec::acp("custom", Command::new("program", ["two words".into()]));
    let registration: AgentRegistration = spec.clone().into();
    assert_eq!(
        serde_json::to_value(&registration).expect("registration"),
        serde_json::to_value(&spec).expect("spec")
    );
    let decoded: AgentRegistration =
        serde_json::from_value(serde_json::to_value(&spec).expect("spec")).expect("custom");
    assert_eq!(decoded.as_custom(), Some(&spec));
    assert!(decoded.managed().is_none());
    assert_eq!(decoded.resolve().expect("resolve custom"), spec);
}

#[test]
fn managed_shapes_forbid_all_overrides_and_unknown_fields() {
    let (_dir, managed) = fixture("a", 1, &AgentSpec::acp("a", Command::new("a", [])));
    let value = serde_json::to_value(&managed).expect("managed JSON");
    let registration: AgentRegistration =
        serde_json::from_value(value.clone()).expect("managed registration");
    assert_eq!(registration.managed(), Some(&managed));
    assert!(registration.as_custom().is_none());
    for (key, extra) in [
        ("kind", json!("acp")),
        ("command", json!({"program":"override"})),
        ("args", json!([])),
        ("env", json!({})),
        ("cancellation_grace_ms", json!(1)),
        ("unknown", json!(true)),
    ] {
        let mut mixed = value.clone();
        mixed[key] = extra;
        assert!(
            serde_json::from_value::<AgentRegistration>(mixed).is_err(),
            "{key}"
        );
    }
    let mut mixed = value;
    mixed["installation"]["command"] = json!("override");
    assert!(serde_json::from_value::<AgentRegistration>(mixed).is_err());
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

#[test]
fn schema_identity_kind_and_launch_validation_are_required() {
    let spec = AgentSpec::acp("a", Command::new("a", []));
    let (_dir, managed) = fixture("a", 2, &spec);
    assert!(matches!(
        managed.resolve(),
        Err(RegistrationError::SchemaVersion(2))
    ));
    let (_dir, managed) = fixture("b", 1, &spec);
    assert!(matches!(
        managed.resolve(),
        Err(RegistrationError::IdMismatch { .. })
    ));
    let (_dir, managed) = fixture("embedded", 1, &AgentSpec::embedded());
    assert!(matches!(managed.resolve(), Err(RegistrationError::NotAcp)));
    let invalid = AgentSpec::acp("a", Command::new("", []));
    let (_dir, managed) = fixture("a", 1, &invalid);
    assert!(matches!(
        managed.resolve(),
        Err(RegistrationError::Validation(_))
    ));
}

#[test]
fn registrations_independently_pin_different_versions() {
    let first = AgentSpec::acp("first", Command::new("/versions/1/agent", []));
    let second = AgentSpec::acp("second", Command::new("/versions/2/agent", []));
    let (_first_dir, first_registration) = fixture("first", 1, &first);
    let (_second_dir, second_registration) = fixture("second", 1, &second);
    let registrations = [
        AgentRegistration::Managed(first_registration.clone()),
        AgentRegistration::Managed(second_registration),
    ];
    assert_eq!(
        resolve_agents(&registrations).expect("independent resolutions"),
        [first, second.clone()]
    );
    fs::write(&first_registration.installation.path, b"changed").expect("tamper first version");
    assert!(resolve_agents(&registrations).is_err());
    assert_eq!(
        registrations[1].resolve().expect("second unaffected"),
        second
    );
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
