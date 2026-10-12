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
fn local_settings_must_match_the_daemon_file() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("settings.json");
    let mut catalog = catalog(path.clone());
    assert!(settings_match(&catalog, &path));
    assert!(!settings_match(
        &catalog,
        &directory.path().join("other.json")
    ));
    catalog.settings_path = None;
    assert!(!settings_match(&catalog, &path));
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

#[test]
fn installed_filter_includes_missing_configured_agents_and_managed_versions() {
    let catalog = catalog("/fixture/settings.json".into());
    let mut entry = catalog.entries[0].clone();
    for status in [Status::NotInstalled, Status::Unsupported] {
        entry.status = status;
        assert!(Filter::All.matches(&entry));
        assert!(!Filter::Installed.matches(&entry));
        assert!(Filter::NotInstalled.matches(&entry));
    }
    for status in [Status::Installed, Status::Missing] {
        entry.status = status;
        assert!(Filter::All.matches(&entry));
        assert!(Filter::Installed.matches(&entry));
        assert!(!Filter::NotInstalled.matches(&entry));
    }
    entry.status = Status::NotInstalled;
    entry.installed_version = Some("1.2.3".into());
    assert!(Filter::Installed.matches(&entry));
    assert!(!Filter::NotInstalled.matches(&entry));
    entry.installed_version = None;
    entry.spec = Some(basr::config::AgentSpec::embedded());
    assert!(Filter::Installed.matches(&entry));
    assert!(!Filter::NotInstalled.matches(&entry));
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn registry_cards_are_searchable_and_leave_custom_configuration_intact(
    cx: &mut gpui_kit::TestAppContext,
) {
    use std::sync::{Arc, Mutex};

    use gpui_kit::{
        component::{Theme, ThemeMode},
        http_client::{FakeHttpClient, Response},
        px, size,
        test::TestWindowExt,
    };
    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("settings.json");
    let initial = r#"{"daemon":{"agents":[{"id":"legacy","kind":"acp","command":{"program":"/missing/legacy-agent","args":[]},"env":{"KEEP":"unchanged"}}]}}"#;
    std::fs::write(&path, initial).expect("custom configuration");
    let image_requests = Arc::new(Mutex::new(Vec::new()));
    let requests = image_requests.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_http_client(FakeHttpClient::create(move |request| {
            requests
                .lock()
                .expect("record image requests")
                .push(request.uri().to_string());
            async {
                Ok(Response::builder()
                    .status(200)
                    .header("Content-Type", "image/svg+xml")
                    .body(
                        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16"/></svg>"#
                            .into(),
                    )
                    .expect("mock image response"))
            }
        }));
    });
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(basr::config::Store::at(path.clone()), window, cx));
        view = Some(page.clone());
        gpui_kit::component::Root::new(page, window, cx)
    });
    let view = view.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            page.apply_catalog(catalog(path.clone()), true, cx);
            page.open_installation(cx);
        });
        window.render_frame(cx);
        window.click("agents-configure", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("agents-card-0").is_some(),
            "missing configured backend stays visible"
        );
        view.update(cx, |page, cx| page.open_installation(cx));
        window.render_frame(cx);
        assert!(window.try_find("agents-registry-codex").is_some());
        assert!(
            window.find("agents-icon-codex").visible(),
            "real agent icon has its own stable layout while loading"
        );
        assert!(window.try_find("agents-registry-opencode").is_some());
        assert!(window.try_find("agents-install-codex").is_some());
        assert!(
            window.try_find("agents-install-pi").is_none(),
            "unsupported backend cannot be installed"
        );
        let card = window.find("agents-registry-codex").bounds();
        let heading = window.find("agents-registry-heading-codex").bounds();
        let details = window.find("agents-registry-details-codex").bounds();
        assert!(
            heading.bottom() <= details.top(),
            "name/actions and description/links occupy separate rows"
        );
        assert!(
            card.size.height <= px(110.),
            "idle catalog cards stay compact"
        );
        assert!(view.read(cx).registry_filter == Filter::All);

        window.click("agents-filter-installed", cx);
        window.render_frame(cx);
        assert!(view.read(cx).registry_filter == Filter::Installed);
        assert!(window.try_find("agents-registry-opencode").is_some());
        assert!(window.try_find("agents-registry-codex").is_none());
        assert!(window.try_find("agents-registry-pi").is_none());
        window.click("agents-filter-not-installed", cx);
        window.render_frame(cx);
        assert!(view.read(cx).registry_filter == Filter::NotInstalled);
        assert!(window.try_find("agents-registry-opencode").is_none());
        assert!(window.try_find("agents-registry-codex").is_some());
        assert!(window.try_find("agents-registry-pi").is_some());
        window.click("agents-filter-all", cx);
        window.render_frame(cx);
        assert!(view.read(cx).registry_filter == Filter::All);
        assert!(window.try_find("agents-registry-opencode").is_some());
        assert!(window.try_find("agents-registry-codex").is_some());
        assert!(window.try_find("agents-registry-pi").is_some());

        window.click("agents-registry-search", cx);
        window.input("Personal", cx);
        window.render_frame(cx);
        assert!(window.try_find("agents-registry-pi").is_some());
        assert!(window.try_find("agents-registry-codex").is_none());
        assert!(window.try_find("agents-registry-opencode").is_none());
        assert!(
            view.read(cx).query.is_empty(),
            "registry search does not overwrite sidebar search"
        );
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        view.update(cx, |page, cx| {
            page.query = "Codex".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("agents-registry-codex").is_some());
        assert!(window.try_find("agents-registry-pi").is_none());
        assert!(!view.read(cx).dirty(cx));
        assert_eq!(
            std::fs::read_to_string(&path).expect("configuration retained"),
            initial
        );
        view.update(cx, |page, cx| {
            page.apply_catalog(catalog(directory.path().join("another.json")), true, cx);
            assert!(!page.local_editable);
            assert!(matches!(
                page.candidate(cx),
                Err(super::super::DraftError::ReadOnly)
            ));
            let mut remote = catalog(path.clone());
            remote.can_install = false;
            let mut custom = remote.entries[0].clone();
            custom.id = "remote-custom".into();
            custom.name = "Remote custom agent".into();
            custom.source = Source::Custom;
            custom.status = Status::Missing;
            remote.entries.push(custom);
            page.query.clear();
            page.apply_catalog(remote, false, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(
            window.try_find("agents-registry-remote-custom").is_some(),
            "remote custom metadata stays visible without a launch spec"
        );
        assert!(
            window
                .try_find("agents-registry-check-remote-custom")
                .is_none(),
            "remote launch commands remain unavailable"
        );
    })
    .expect("fixture catalogue");
    cx.run_until_parked();
    assert_eq!(
        *image_requests.lock().expect("image requests"),
        ["https://example.com/monochrome.svg"],
        "dark mode loads and shares the original artwork"
    );

    cx.update_window(window.into(), |_, window, cx| {
        Theme::change(ThemeMode::Light, Some(window), cx);
        window.render_frame(cx);
        assert!(
            window.find("agents-icon-codex-svg").visible(),
            "light mode retains the real SVG logo"
        );
    })
    .expect("light theme adaptation");
    cx.run_until_parked();
    assert_eq!(
        *image_requests.lock().expect("image requests"),
        ["https://example.com/monochrome.svg"],
        "theme changes reuse the original SVG URI cache without another request"
    );
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn authentication_failure_has_one_owner_in_catalog_and_advanced_configuration(
    cx: &mut gpui_kit::TestAppContext,
) {
    use basr::{
        authentication::{Arguments, Execution, Method, Source as AuthenticationSource, Terminal},
        config::{AgentSpec, Store, agent::Command},
    };
    use gpui_kit::{component::Root, px, size, test::TestWindowExt};
    use std::collections::BTreeMap;

    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("settings.json");
    let store = Store::at(path.clone());
    let spec = AgentSpec::acp("codex", Command::new("/fixture/agent", []));
    let mut document = store.load().expect("fixture document");
    store
        .save_agents(&mut document, vec![spec.clone()])
        .expect("fixture agent configuration");
    let method = Method {
        id: "terminal".into(),
        name: "Agent account".into(),
        description: Some("Use the account owned by this Agent".into()),
        raw: serde_json::json!({"id":"terminal"}),
        source: AuthenticationSource::Initialize,
        execution: Execution::Terminal(Terminal {
            args: vec!["login".into()],
            env: BTreeMap::new(),
            arguments: Arguments::Append,
            command: None,
        }),
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(900.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store, window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("settings page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            let mut registry = catalog(path.clone());
            registry.entries.truncate(1);
            registry.entries[0].icon.clear();
            registry.entries[0].spec = Some(spec.clone());
            registry.entries[0].status = Status::Installed;
            page.apply_catalog(registry, true, cx);
            page.apply_check(
                &spec,
                super::super::check::Status::AuthenticationRequired {
                    program: "/fixture/agent".into(),
                    methods: vec![method],
                },
            );
            page.open_installation(cx);
        });
        window.render_frame(cx);
        let windows = cx.windows();
        window.click("authentication-method-0", cx);
        window.render_frame(cx);
        assert_eq!(
            cx.windows(),
            windows,
            "terminal authentication starts in place"
        );
    })
    .expect("begin authentication through the catalog button");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("authentication-error").visible());
        assert!(window.try_find("agents-registry-status-codex").is_none());
        assert!(window.try_find("agents-operation-codex").is_none());
        assert!(matches!(
            view.read(cx).status(&spec),
            Some(super::super::check::Status::Failed(_))
        ));
        assert_eq!(
            view.read(cx).check_detail(&spec),
            "选择认证方式以开始对话。"
        );

        view.update(cx, |page, cx| {
            page.operations.insert(
                "codex".into(),
                Operation {
                    message: "安装完成".into(),
                    busy: false,
                    _task: None,
                },
            );
            cx.notify();
        });
        window.render_frame(cx);
        assert!(
            window.find("agents-operation-codex").visible(),
            "independent installation feedback remains visible alongside authentication"
        );
        window.click("agents-configure", cx);
        window.render_frame(cx);
        assert!(window.find("authentication-error").visible());
        assert!(window.try_find("agents-status-0").is_none());
    })
    .expect("single authentication error in both settings views");
}
