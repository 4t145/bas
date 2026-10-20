use super::*;

#[cfg(feature = "gui-tests")]
mod stream;

fn catalog(path: std::path::PathBuf) -> Catalog {
    Catalog {
        entries: vec![
            Entry {
                id: "codex".into(),
                name: "Codex ACP".into(),
                description: "Use Codex through ACP".into(),
                version: "1.2.3".into(),
                repository: "https://example.com/codex".into(),
                license: "Apache-2.0".into(),
                license_url: "https://example.com/license".into(),
                icon: "https://example.com/monochrome.svg".into(),
                installed_version: None,
                source: Source::Registry,
                status: Status::NotInstalled,
                message: String::new(),
                spec: None,
            },
            Entry {
                id: "pi".into(),
                name: "Pi ACP".into(),
                description: "Personal assistant".into(),
                version: "2.0".into(),
                repository: "https://example.com/pi".into(),
                license: "MIT".into(),
                license_url: "https://example.com/pi/license".into(),
                icon: String::new(),
                installed_version: None,
                source: Source::Registry,
                status: Status::Unsupported,
                message: "Platform unavailable".into(),
                spec: None,
            },
            Entry {
                id: "opencode".into(),
                name: "OpenCode".into(),
                description: "Open source coding agent".into(),
                version: "3.0".into(),
                repository: "https://example.com/opencode".into(),
                license: "MIT".into(),
                license_url: "https://example.com/opencode/license".into(),
                icon: String::new(),
                installed_version: Some("3.0".into()),
                source: Source::Registry,
                status: Status::Installed,
                message: String::new(),
                spec: None,
            },
        ],
        warning: Some("Using cached registry".into()),
        settings_path: Some(path),
        can_install: true,
    }
}

#[test]
fn registry_search_matches_names_descriptions_and_ids() {
    let catalog = catalog("/fixture/settings.json".into());
    assert!(matches_query(&catalog.entries[0], "CODEX acp"));
    assert!(matches_query(&catalog.entries[1], "personal"));
    assert!(!matches_query(&catalog.entries[0], "Pi"));
    assert!(matches_query(&catalog.entries[0], "example.com/codex"));
    assert!(!matches_query(&catalog.entries[0], "codex unavailable"));
}
