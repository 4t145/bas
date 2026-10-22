use super::*;
use gpui_kit::{Context, Render, TestAppContext, size, test::TestWindowExt};

struct Probe(Entry);

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(Item {
            entry: self.0.clone(),
            status: None,
            status_hint: String::new(),
            feedback: Some(self.0.message.clone()),
            actions: action("agent-edit", IconName::Settings, "编辑").into_any_element(),
            content: None,
        })
    }
}

#[gpui_kit::test]
fn agent_card_keeps_feedback_links_and_actions_without_copy(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let entry = Entry {
        id: "custom".into(),
        name: "Custom ACP".into(),
        description: "Agent description".into(),
        version: String::new(),
        repository: "https://example.com/agent".into(),
        license: "MIT".into(),
        license_url: "https://example.com/license".into(),
        icon: String::new(),
        installed_version: None,
        source: Source::Custom,
        status: Status::Installed,
        message: "自定义配置".into(),
        spec: None,
    };
    let window = cx.open_window(size(px(640.), px(240.)), |window, cx| {
        let view = cx.new(|_| Probe(entry));
        crate::theme::window_root(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agents-operation-custom").visible());
        assert_eq!(
            window.find("agents-operation-custom-message").label(),
            Some("自定义配置"),
        );
        assert!(window.try_find("agents-operation-custom-copy").is_none());
        for id in [
            "agent-edit",
            "agents-repository-custom",
            "agents-license-custom",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
    })
    .expect("agent card");
}
