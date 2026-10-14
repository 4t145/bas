use super::*;

#[test]
fn appearance_save_keeps_agents_written_since_the_form_was_opened() {
    use bas::config::agent::{AgentSpec, Command};
    let directory = tempfile::tempdir().expect("isolated settings");
    let loader = Loader::new(Store::at(directory.path().join("settings.json")), None);
    let mut appearance = loader.document().expect("appearance baseline");
    let mut agents = loader.store.load().expect("agents baseline");
    let catalog = vec![bas::config::AgentRegistration::from(AgentSpec::acp(
        "custom",
        Command::new("test-acp", vec!["argument with spaces".into()]),
    ))];
    loader
        .store
        .save_agents(&mut agents, catalog.clone())
        .expect("save agents");
    let candidate = Preferences {
        theme_source: ThemeSource::Builtin,
        ui_font_size: Some(19.),
        ..Default::default()
    };
    loader
        .save(&mut appearance, candidate.clone(), true)
        .expect("save appearance");
    let saved = loader.store.load().expect("unified file").settings;
    assert_eq!(saved.appearance, candidate);
    assert_eq!(saved.daemon.agents, catalog);
}

#[test]
fn safe_save_roundtrip_validation_conflict_and_inheritance() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("settings.json");
    let loader = Loader::new(Store::at(path.clone()), None);
    let mut document = loader.document().expect("missing config");
    let candidate = Preferences {
        theme_source: ThemeSource::Builtin,
        ui_font_size: Some(21.),
        ..Preferences::default()
    };
    loader
        .save(&mut document, candidate.clone(), true)
        .expect("save");
    assert_eq!(loader.document().expect("read").preferences, candidate);
    let before = std::fs::read(&path).expect("bytes");
    for value in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(
            loader
                .save(
                    &mut document,
                    Preferences {
                        ui_font_size: Some(value),
                        ..candidate.clone()
                    },
                    true
                )
                .is_err()
        );
        assert_eq!(std::fs::read(&path).expect("bytes"), before);
    }
    let inherited = Preferences {
        ui_font_size: None,
        ..candidate.clone()
    };
    loader
        .save(&mut document, inherited.clone(), true)
        .expect("inherit");
    assert_eq!(loader.document().expect("read").preferences, inherited);
    std::fs::write(&path, "{}").expect("external edit");
    assert!(loader.save(&mut document, candidate, true).is_err());
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "{}");
    std::fs::write(&path, "{broken").expect("invalid external config");
    assert!(loader.document().is_err());
}

#[test]
fn save_io_failure_preserves_document() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("settings.json");
    let loader = Loader::new(Store::at(path.clone()), None);
    let mut document = loader.document().expect("missing");
    std::fs::create_dir(&path).expect("directory at target");
    assert!(
        loader
            .save(
                &mut document,
                Preferences {
                    theme_source: ThemeSource::Builtin,
                    ..Preferences::default()
                },
                true
            )
            .is_err()
    );
    assert_eq!(document.preferences, Preferences::default());
    assert!(document.settings.source_paths().is_empty());
}

#[cfg(unix)]
#[test]
fn save_rejects_symlinks_without_modifying_target() {
    let dir = tempfile::tempdir().expect("directory");
    let target = dir.path().join("managed.json");
    let path = dir.path().join("settings.json");
    std::fs::write(&target, r#"{"appearance":{"theme_source":"builtin"}}"#)
        .expect("managed config");
    std::os::unix::fs::symlink(&target, &path).expect("symlink");
    let loader = Loader::new(Store::at(path.clone()), None);
    let mut document = loader.document().expect("read link");
    let candidate = document.preferences.clone();
    let error = loader
        .save(&mut document, candidate, true)
        .err()
        .expect("reject link");
    assert!(
        error.downcast_ref::<bas::config::Error>().is_some(),
        "shared settings store reports the refused write"
    );
    assert!(path.is_symlink());
    assert_eq!(
        std::fs::read_to_string(target).expect("unchanged"),
        r#"{"appearance":{"theme_source":"builtin"}}"#
    );
}

fn snapshot(preferences: Preferences, files: Files) -> Snapshot {
    Snapshot {
        preferences,
        files,
        source_description: "/test/zed/settings.json".into(),
    }
}

#[test]
fn save_between_polls_invalidates_the_pre_save_deduplication_snapshot() {
    let directory = tempfile::tempdir().expect("isolated config directory");
    let path = directory.path().join("settings.json");
    let original = r#"{"appearance":{"theme_source":"builtin","content_font_size":19}}"#;
    std::fs::write(&path, original).expect("original config");
    let loader = Loader::new(Store::at(path.clone()), None);
    let mut reload = Reload::default();
    let initial = reload
        .update(loader.read().expect("initial read"), true)
        .expect("initial resolve")
        .expect("initial config applied");
    assert_eq!(initial.content_font_size, 19.);

    // No watcher read occurs between the immediate save and the external restore.
    let mut document = loader.document().expect("settings document");
    let edited = Preferences {
        content_font_size: Some(23.),
        ..document.preferences.clone()
    };
    let saved = loader.save(&mut document, edited, true).expect("save");
    assert_eq!(saved.content_font_size, 23.);
    std::fs::write(&path, original).expect("external restore");
    reload.invalidate_on_save(1);
    let restored = reload
        .update(loader.read().expect("restored read"), true)
        .expect("restored resolve")
        .expect("restore must replace the locally saved appearance");
    assert_eq!(restored.content_font_size, 19.);
    reload.invalidate_on_save(1);
    assert!(
        reload
            .update(loader.read().expect("unchanged read"), true)
            .expect("unchanged resolve")
            .is_none(),
        "a generation is invalidated only once"
    );
}

#[test]
fn invalid_reload_retains_the_last_successful_snapshot() {
    let mut reload = Reload::default();
    let valid = snapshot(Preferences::default(), Files::default());
    assert!(
        reload
            .update(valid.clone(), true)
            .expect("initial resolve")
            .is_some()
    );
    let invalid = snapshot(
        Preferences::default(),
        Files {
            zed_settings: Some("{".into()),
            ..Files::default()
        },
    );
    let error = reload.update(invalid, true).err().expect("invalid JSON");
    assert!(format!("{error:#}").contains("/test/zed/settings.json"));
    assert!(
        reload
            .update(valid.clone(), true)
            .expect("retained snapshot")
            .is_none()
    );
    let invalid = snapshot(
        Preferences {
            content_font_size: Some(0.),
            ..Preferences::default()
        },
        Files::default(),
    );
    assert!(reload.update(invalid, true).is_err());
    assert!(
        reload
            .update(valid, true)
            .expect("retained preferences")
            .is_none()
    );
}

#[test]
fn builtin_source_does_not_parse_unrelated_broken_desktop_files() {
    let preferences = Preferences {
        theme_source: ThemeSource::Builtin,
        theme_mode: ThemeMode::Light,
        ui_font_family: Some("UI".into()),
        mono_font_family: Some("Code".into()),
        font_fallbacks: Some(vec![]),
        ui_font_size: Some(18.),
        content_font_size: Some(16.),
        ..Preferences::default()
    };
    let effective = snapshot(
        preferences,
        Files {
            zed_theme: Some("{".into()),
            zed_settings: Some("{".into()),
            dms_colors: Some("{".into()),
            dms_settings: Some("{".into()),
        },
    )
    .resolve(true)
    .expect("desktop files are not selected");
    assert!(!effective.desktop.config.mode.is_dark());
    assert_eq!(effective.desktop.config.font_size, Some(18.));
    assert_eq!(effective.content_font_size, 16.);
}

#[test]
fn explicit_mode_overrides_desktop_selection() {
    let files = Files {
        zed_theme: Some(
            r#"{"themes":[
            {"name":"Light","appearance":"light","style":{}},
            {"name":"Dark","appearance":"dark","style":{}}
        ]}"#
            .into(),
        ),
        zed_settings: Some(r#"{"theme":"Dark"}"#.into()),
        ..Files::default()
    };
    let preferences = Preferences {
        theme_mode: ThemeMode::Light,
        ..Preferences::default()
    };
    let effective = snapshot(preferences, files)
        .resolve(true)
        .expect("valid explicit mode");
    assert!(!effective.desktop.config.mode.is_dark());
}

#[test]
fn explicit_mode_rejects_a_named_variant_with_the_wrong_appearance() {
    let files = Files {
        zed_theme: Some(
            r#"{"themes":[
                {"name":"Light","appearance":"light","style":{}},
                {"name":"Dark","appearance":"dark","style":{}}
            ]}"#
            .into(),
        ),
        zed_settings: Some(r#"{"theme":{"mode":"dark","light":"Dark","dark":"Light"}}"#.into()),
        ..Files::default()
    };
    for (mode, system_dark, expected_dark) in [
        (ThemeMode::Light, true, false),
        (ThemeMode::Dark, false, true),
        (ThemeMode::System, false, false),
        (ThemeMode::System, true, true),
    ] {
        let effective = snapshot(
            Preferences {
                theme_mode: mode,
                ..Preferences::default()
            },
            files.clone(),
        )
        .resolve(system_dark)
        .expect("a variant matching the explicit mode exists");
        assert_eq!(effective.desktop.config.mode.is_dark(), expected_dark);
    }
}

#[test]
fn malformed_style_or_unicode_color_returns_an_error_without_replacing_the_snapshot() {
    let mut reload = Reload::default();
    let valid = snapshot(Preferences::default(), Files::default());
    assert!(
        reload
            .update(valid.clone(), true)
            .expect("initial valid snapshot")
            .is_some()
    );
    for style in [
        serde_json::json!("invalid"),
        serde_json::json!([]),
        serde_json::Value::Null,
        serde_json::json!({"editor.background": "#😀😀"}),
        serde_json::json!({"editor.background": "#zzzzzzzz"}),
    ] {
        let family = serde_json::json!({
            "themes": [{"name": "Dark", "appearance": "dark", "style": style}]
        });
        let invalid = snapshot(
            Preferences::default(),
            Files {
                zed_theme: Some(family.to_string()),
                ..Files::default()
            },
        );
        assert!(reload.update(invalid, true).is_err());
        assert!(
            reload
                .update(valid.clone(), true)
                .expect("last valid snapshot survives rejected input")
                .is_none()
        );
    }
}

#[test]
fn preference_file_reads_distinguish_missing_invalid_and_io_errors() {
    let directory = tempfile::tempdir().expect("test directory");
    let path = directory.path().join("settings.json");
    assert!(source::read(&path).expect("missing is optional").is_none());
    assert!(!path.exists());
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","ui_font_size":19}}"#,
    )
    .expect("write preferences");
    let loader = Loader::new(Store::at(path.clone()), None);
    let first = loader.read().expect("read valid preferences");
    let mut reload = Reload::default();
    assert_eq!(
        reload
            .update(first.clone(), true)
            .expect("resolve")
            .expect("changed")
            .desktop
            .config
            .font_size,
        Some(19.)
    );
    std::fs::write(&path, r#"{"appearance":{"ui_font_size":0}}"#)
        .expect("write invalid preferences");
    let error = loader.read().err().expect("invalid preferences rejected");
    assert!(format!("{error:#}").contains(path.to_str().expect("path")));
    assert!(
        reload
            .update(first, true)
            .expect("last valid is unchanged")
            .is_none()
    );
    std::fs::remove_file(&path).expect("remove test file");
    std::fs::create_dir(&path).expect("directory cannot be read as JSON");
    let error = loader.read().err().expect("IO error is not missing");
    assert!(format!("{error:#}").contains(path.to_str().expect("path")));
}

#[test]
fn cli_theme_path_wins_and_preferences_paths_are_relative_to_their_file() {
    let preferences = Preferences {
        theme_file: Some("themes/custom.json".into()),
        ..Preferences::default()
    };
    let mut loader = Loader::new(Store::at("/test/bas/settings.json".into()), None);
    assert_eq!(
        loader.theme_path(&preferences),
        Some("/test/bas/themes/custom.json".into())
    );
    loader.theme_file = Some("cli-theme.json".into());
    assert_eq!(
        loader.theme_path(&preferences),
        Some("cli-theme.json".into())
    );
    let preferences = Preferences {
        theme_file: Some("/absolute/theme.json".into()),
        ..preferences
    };
    loader.theme_file = None;
    assert_eq!(
        loader.theme_path(&preferences),
        Some("/absolute/theme.json".into())
    );
}
