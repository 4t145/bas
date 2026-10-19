use super::*;

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
