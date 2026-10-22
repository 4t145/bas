use super::*;
use bas::config::{AgentSpec, agent::Command};
use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt};

fn snapshot() -> pb::AcpConfiguration {
    pb::AcpConfiguration {
        profile: "server-a".into(),
        revision: "revision-a".into(),
        policy_json: serde_json::to_vec(&AcpLifecycle::default()).expect("policy"),
        registrations_json: serde_json::to_vec(&vec![
            AgentRegistration::from(AgentSpec::embedded()),
            AgentSpec::acp("a", Command::new("unused", [])).into(),
            AgentSpec::acp("b", Command::new("unused", [])).into(),
        ])
        .expect("agents"),
    }
}

#[gpui_kit::test]
fn form_preserves_disable_override_and_rejects_invalid_reserves(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut page = None;
    let window = cx.open_window(size(px(960.), px(800.)), |window, cx| {
        let entity = cx.new(|cx| Page::new(window, cx));
        entity.update(cx, |this, cx| {
            this.accept(snapshot(), window, cx).expect("snapshot")
        });
        page = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let page = page.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("acp-policy-page").visible());
        assert!(window.find("acp-policy-pool-a").visible());
        assert!(window.try_find("acp-policy-pool-embedded").is_none());
        page.update(cx, |this, cx| {
            assert!(!this.dirty(cx));
            this.idle
                .update(cx, |input, cx| input.set_value("", window, cx));
            this.size
                .update(cx, |input, cx| input.set_value("0", window, cx));
            this.default_agent = Some("a".into());
            this.pools[0]
                .1
                .update(cx, |input, cx| input.set_value("0", window, cx));
            this.pools[1]
                .1
                .update(cx, |input, cx| input.set_value("2", window, cx));
            let value = this.candidate(cx).expect("valid draft");
            assert_eq!(value.idle_timeout_secs, None);
            assert_eq!(value.pool_size, 0);
            assert_eq!(value.pools.get(&"a".into()), Some(&0));
            assert_eq!(value.pools.get(&"b".into()), Some(&2));
            this.pools[1]
                .1
                .update(cx, |input, cx| input.set_value("33", window, cx));
            assert!(this.candidate(cx).is_err());
            this.pools[1]
                .1
                .update(cx, |input, cx| input.set_value("", window, cx));
            this.idle
                .update(cx, |input, cx| input.set_value("0", window, cx));
            assert!(this.candidate(cx).is_err());
        });
    })
    .expect("policy form");
}

#[gpui_kit::test]
fn disconnected_page_never_loads_local_preferences(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut entity = None;
    let window = cx.open_window(size(px(960.), px(800.)), |window, cx| {
        let page = cx.new(|cx| Page::new(window, cx));
        page.update(cx, |this, cx| {
            this.open(window, cx);
            assert!(this.baseline.is_none());
            assert!(this.policy.is_none());
            assert!(!this.message.is_empty());
            assert!(this.disabled());
        });
        entity = Some(page.clone());
        Root::new(page, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("acp-policy-save").visible());
        window.click("acp-policy-save", cx);
        let page = entity.as_ref().expect("page").read(cx);
        assert!(page.task.is_none());
        assert!(page.baseline.is_none());
    })
    .expect("disconnected state");
}
