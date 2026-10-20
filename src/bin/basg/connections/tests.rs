use super::*;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn saved_names_match_all_credentials_and_prefer_the_selected_profile(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings directory");
    cx.update(|cx| {
        let profiles = cx.new(|_| Profiles::new(Store::at(directory.path().join("settings.json"))));
        profiles.update(cx, |profiles, cx| {
            let mut first = ConnectionProfile {
                id: "first".into(),
                name: "Production".into(),
                note: None,
                target: "mtls://hostname".parse().expect("target"),
                tls: Some(bas::service::tls::ClientTls {
                    ca: "/ca.pem".into(),
                    certificate: "/client.pem".into(),
                    private_key: "/client.key".into(),
                    server_name: None,
                }),
            };
            profiles.put(first.clone(), cx).expect("first");
            let options = first.options();
            first.id = "second".into();
            first.name = "Alternate name".into();
            profiles.put(first.clone(), cx).expect("second");
            assert_eq!(profiles.matching(&options).unwrap().name, "Production");
            profiles.select("second", cx).expect("select");
            assert_eq!(profiles.matching(&options).unwrap().name, "Alternate name");
            for field in ["ca", "certificate", "private_key", "server_name"] {
                let mut alternate = options.clone();
                let tls = alternate.tls.as_mut().expect("TLS");
                match field {
                    "ca" => tls.ca = "/other-ca.pem".into(),
                    "certificate" => tls.certificate = "/other-client.pem".into(),
                    "private_key" => tls.private_key = "/other-client.key".into(),
                    _ => tls.server_name = Some("other.example".into()),
                }
                assert!(profiles.matching(&alternate).is_none(), "{field}");
            }
        });
    });
}

#[test]
fn invalid_saved_tls_configuration_never_falls_back_to_local() {
    let directory = tempfile::tempdir().expect("settings directory");
    let store = Store::at(directory.path().join("settings.json"));
    std::fs::write(
        store.path(),
        r#"{
        "frontend": {
            "selected": "secure",
            "connections": [{"id":"secure","name":"Secure","target":"mtls://server:7338"}]
        }
    }"#,
    )
    .expect("incomplete TLS configuration");
    let profiles = Profiles::new(store);
    assert!(profiles.startup_target(None).is_err());
    assert!(
        profiles
            .startup_target(Some("mtls://server:7338".parse().expect("target")))
            .is_err()
    );
    assert_eq!(
        profiles
            .startup_target(Some(Target::default()))
            .expect("deliberate local override"),
        Target::default(),
    );
}

#[gpui_kit::test]
fn profiles_persist_selection_without_overwriting_cli_override_or_other_sections(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::at(directory.path().join("settings.json"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        crate::agents::init(Target::default(), runtime.handle().clone(), cx);
        let profiles = cx.new(|_| Profiles::new(store.clone()));
        profiles.update(cx, |profiles, cx| {
            assert_eq!(
                profiles.startup_target(None).expect("default"),
                Target::default()
            );
            let target = profiles
                .put(
                    ConnectionProfile {
                        id: "remote".into(),
                        name: "Remote".into(),
                        note: None,
                        target: "tcp://192.0.2.1".parse().unwrap(),
                        tls: None,
                    },
                    cx,
                )
                .unwrap();
            assert_eq!(
                profiles.startup_target(None).expect("default"),
                Target::default(),
                "saving does not select"
            );
            profiles.select("remote", cx).expect("explicit selection");
            manage(target.clone(), cx);
            assert_eq!(current(cx), Some(target.clone()));
            assert_eq!(
                Profiles::new(store.clone())
                    .startup_target(None)
                    .expect("saved selection"),
                target
            );
            assert_eq!(
                profiles
                    .startup_target(Some(Target::default()))
                    .expect("explicit override"),
                Target::default()
            );
            let mut outside = store.load().unwrap();
            let appearance = outside.settings.appearance.clone();
            // A concurrent same-section change must not silently overwrite the selection.
            let mut frontend = outside.settings.frontend.clone();
            frontend.connections[0].name = "Changed externally".into();
            store.save_frontend(&mut outside, frontend).unwrap();
            assert!(profiles.select("local", cx).is_err());
            assert_eq!(current(cx), Some(target));
            profiles.reload(cx);
            let local = profiles.select("local", cx).unwrap();
            manage(local, cx);
            assert_eq!(current(cx), Some(Target::default()));
            // Other sections remain available to their own section-scoped writers.
            assert_eq!(store.load().unwrap().settings.appearance, appearance);
        });
    });
}

#[gpui_kit::test]
fn deleting_profile_preserves_a_valid_default_and_never_deletes_the_last_one(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::at(directory.path().join("settings.json"));
    cx.update(|cx| {
        let profiles = cx.new(|_| Profiles::new(store.clone()));
        profiles.update(cx, |profiles, cx| {
            assert!(profiles.remove("local", cx).is_err());
            profiles
                .put(
                    ConnectionProfile {
                        id: "other".into(),
                        name: "Other".into(),
                        note: None,
                        target: bas::service::Target::Pipe("bas-other".into()),
                        tls: None,
                    },
                    cx,
                )
                .unwrap();
            assert_eq!(profiles.remove("other", cx).unwrap(), Target::default());
            assert_eq!(profiles.selected().unwrap().id, "local");
            assert_eq!(store.load().unwrap().settings.frontend.connections.len(), 1);
        });
    });
}

#[gpui_kit::test]
fn opening_a_registered_backend_window_does_not_retarget_management_or_replay_selection(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{IntoElement, Render, Window, component::Root, div, px, size};

    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    cx.update(gpui_kit::init);
    let first = cx.open_window(size(px(800.), px(600.)), |window, cx| {
        Root::new(cx.new(|_| Empty), window, cx)
    });
    let second = cx.open_window(size(px(800.), px(600.)), |window, cx| {
        Root::new(cx.new(|_| Empty), window, cx)
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let a = Target::default();
    let b: Target = bas::service::Target::Pipe("another-backend".into()).into();
    cx.update(|cx| {
        crate::agents::init(b.clone(), runtime.handle().clone(), cx);
        init_windows(runtime.handle().clone(), "/unused".into(), false, cx);
        register(a.clone(), first, cx);
        register(b.clone(), second, cx);
        cx.update_global::<crate::agents::Changes, _>(|changes, _| {
            changes.server = Some(b.clone());
            changes.select = Some("stale-agent-selection".into());
        });
        assert_eq!(
            open_for(a.clone(), cx).expect("reuse A").window_id(),
            first.window_id()
        );
        assert_eq!(
            current(cx),
            Some(b.clone()),
            "opening a pinned window is not management switching"
        );
        assert!(cx.global::<crate::agents::Changes>().select.is_none());
        assert_eq!(
            open_current(cx).expect("reuse B").window_id(),
            second.window_id()
        );
        assert_eq!(
            open_for(a, cx).expect("A remains available").window_id(),
            first.window_id()
        );
    });
}
