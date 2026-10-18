use super::*;
use gpui_kit::{AppContext, Context, Render, TestAppContext, size, test::TestWindowExt};

fn entry() -> Entry {
    Entry {
        id: "codex".into(),
        name: "Codex".into(),
        description: "ACP adapter".into(),
        version: "2.0".into(),
        repository: String::new(),
        license: String::new(),
        license_url: String::new(),
        icon: String::new(),
        installed_version: Some("1.0".into()),
        source: Source::Registry,
        status: Status::Installed,
        message: "安装完成".into(),
        spec: None,
    }
}

#[test]
fn entry_methods_prefer_the_installed_version_and_hide_missing_versions() {
    let mut entry = entry();
    assert_eq!(entry.displayed_version(), Some("1.0"));
    entry.installed_version = Some(String::new());
    assert_eq!(entry.displayed_version(), Some("2.0"));
    entry.installed_version = None;
    entry.source = Source::Custom;
    assert_eq!(entry.displayed_version(), None);
    entry.source = Source::Registry;
    assert_eq!(entry.displayed_version(), Some("2.0"));
    entry.version.clear();
    assert_eq!(entry.displayed_version(), None);
}

struct Actions;
impl Render for Actions {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gds_uim::el! {
            #div { flex; gap_1; } [
                #action("verify", $IconName::RefreshCw, $crate::i18n::tr!("agent-check")),
                #action("chat", $IconName::MessageCircle, $crate::i18n::tr!("agent-start-chat")),
                #action("remove", $IconName::Trash, $crate::i18n::tr!("agent-mark-remove")),
            ]
        }
    }
}

#[gpui_kit::test]
fn item_actions_use_one_compact_scale_and_accessible_hover_names(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let window = cx.open_window(size(px(500.), px(200.)), |_, _| Actions);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for (id, name) in [
            ("verify", crate::i18n::tr!("agent-check")),
            ("chat", crate::i18n::tr!("agent-start-chat")),
            ("remove", crate::i18n::tr!("agent-mark-remove")),
        ] {
            let button = window.find(id);
            assert_eq!(button.role(), Some(gpui_kit::Role::Button));
            assert_eq!(button.label(), Some(name));
            assert_eq!(
                button.bounds().size,
                size(
                    px(crate::ui::COMPACT_CONTROL),
                    px(crate::ui::COMPACT_CONTROL)
                )
            );
        }
    })
    .expect("icon actions render consistently");
}

#[gpui_kit::test]
fn installed_agents_are_managed_in_settings_not_offered_for_installation(cx: &mut TestAppContext) {
    use super::super::{Check, window as onboarding};
    use bas::config::{AgentSpec, Store, agent::Command};
    use gpui_kit::component::Root;

    let directory = tempfile::tempdir().expect("backend profile");
    let path = directory.path().join("settings.json");
    let store = Store::at(path.clone());
    let spec = AgentSpec::acp("codex", Command::new("/managed/acp", []));
    let registration = super::super::tests::managed_fixture(directory.path(), &spec);
    let mut document = store.load().expect("profile");
    store
        .save_agents(&mut document, vec![registration])
        .expect("managed registration");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut agents = None;
    let configuration = cx.open_window(size(px(900.), px(720.)), |window, cx| {
        let view = cx.new(|cx| Agents::new(store, window, cx));
        agents = Some(view.clone());
        Root::new(view, window, cx)
    });
    let agents = agents.expect("shared agent state");
    let installation = cx.update(|cx| {
        agents.update(cx, |agents, cx| {
            let mut entry = entry();
            entry.spec = Some(spec.clone());
            agents.apply_catalog(
                bas::management::Catalog {
                    entries: vec![entry],
                    warning: None,
                    settings_path: Some(path),
                    can_install: true,
                },
                true,
                cx,
            );
            agents.checks.insert(
                spec.id.clone(),
                Check {
                    spec: spec.clone(),
                    status: crate::agents::Status::Connected("/managed/acp".into()),
                    completed_at: None,
                    _task: None,
                },
            );
        });
        onboarding::open(agents.clone(), cx).expect("installation surface")
    });
    cx.update_window(configuration.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("agents-source-registry").label(),
            Some("ACP 注册目录")
        );
        assert!(window.try_find("agents-operation-codex").is_none());
        assert_eq!(
            window.find("agents-item-version-codex").label(),
            Some("1.0")
        );
        assert!(window.try_find("agents-item-status-codex").is_none());
        assert!(window.find("agent-logout-codex").visible());
        assert!(window.find("agents-registry-start-chat-codex").visible());
        let check = "agents-registry-check-codex";
        window.hover("agents-item-name-codex", cx);
        assert_eq!(window.find(format!("{check}-result")).label(), Some("可用"));
        assert!(window.find(format!("{check}-result")).visible());
        assert!(!window.find(format!("{check}-retry")).visible());
        window.hover(check, cx);
        assert!(!window.find(format!("{check}-result")).visible());
        assert!(window.find(format!("{check}-retry")).visible());
        let remove = window.find("agents-remove-codex").bounds();
        let chat = window.find("agents-registry-start-chat-codex").bounds();
        assert_eq!(remove.size, chat.size);
        assert_eq!(remove.top(), chat.top());
        assert!(remove.left() > chat.right());
    })
    .expect("installed management actions");
    cx.update_window(installation.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agents-item-codex").is_none());
        assert!(window.try_find("agents-remove-codex").is_none());
        assert!(window.find("agents-add-custom").visible());
    })
    .expect("installation only offers new agents");
    assert!(agents.read_with(cx, |agents, _| agents.registry.verified_local()));
}
