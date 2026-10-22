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
        assert_eq!(
            window.find("session-management-title").label(),
            Some("会话管理")
        );
        assert!(window.try_find("acp-policy-save").is_none());
        assert!(window.try_find("acp-policy-reload").is_none());
        assert!(window.find("acp-policy-add-pool").visible());
        assert!(window.find("acp-policy-pools-empty").visible());
        assert!(window.try_find("acp-policy-pool-a").is_none());
        assert!(window.try_find("acp-policy-pool-embedded").is_none());
        page.update(cx, |this, cx| {
            assert!(!this.dirty(cx));
            assert_eq!(
                this.acp_agents().cloned().collect::<Vec<_>>(),
                vec!["a".into(), "b".into()]
            );
            this.add_pool("a".into(), window, cx);
            this.add_pool("a".into(), window, cx);
            this.add_pool("embedded".into(), window, cx);
            this.add_pool("missing".into(), window, cx);
            assert_eq!(
                this.pools.len(),
                1,
                "only eligible, unique agents can be added"
            );
            assert_eq!(
                this.candidate(cx).expect("new pool").pools.get(&"a".into()),
                Some(&1)
            );
            assert_eq!(this.pool_candidates(), vec!["b".into()]);
            this.add_pool("b".into(), window, cx);
            assert!(this.pool_candidates().is_empty());
            this.idle
                .update(cx, |input, cx| input.set_value("", window, cx));
            this.size
                .update(cx, |input, cx| input.set_value("0", window, cx));
            this.default_agent = Some("a".into());
            this.pools[0]
                .size
                .update(cx, |input, cx| input.set_value("0", window, cx));
            this.pools[1]
                .size
                .update(cx, |input, cx| input.set_value("2", window, cx));
            let value = this.candidate(cx).expect("valid draft");
            assert_eq!(value.idle_timeout_secs, None);
            assert_eq!(value.pool_size, 0);
            assert_eq!(value.pools.get(&"a".into()), Some(&0));
            assert_eq!(value.pools.get(&"b".into()), Some(&2));
            this.pools[1]
                .size
                .update(cx, |input, cx| input.set_value("33", window, cx));
            assert!(this.candidate(cx).is_err());
            this.pools[1]
                .size
                .update(cx, |input, cx| input.set_value("", window, cx));
            assert!(
                this.candidate(cx).is_err(),
                "an unfinished count is not a removal"
            );
            this.idle
                .update(cx, |input, cx| input.set_value("0", window, cx));
            assert!(this.candidate(cx).is_err());
        });
    })
    .expect("policy form");
}

#[gpui_kit::test]
fn explicit_pools_load_without_empty_rows_and_removal_restores_automatic_policy(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut saved = snapshot();
    let policy = AcpLifecycle {
        default_agent: Some("a".into()),
        pools: std::collections::BTreeMap::from([("a".into(), 0)]),
        ..Default::default()
    };
    saved.policy_json = serde_json::to_vec(&policy).expect("saved explicit override");
    let mut page = None;
    let window = cx.open_window(size(px(960.), px(800.)), |window, cx| {
        let entity = cx.new(|cx| Page::new(window, cx));
        entity.update(cx, |page, cx| {
            page.accept(saved.clone(), window, cx).expect("load")
        });
        page = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let page = page.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("acp-policy-pool-a").visible());
        assert!(window.try_find("acp-policy-pool-b").is_none());
        assert!(window.try_find("acp-policy-pools-empty").is_none());
        assert_eq!(page.read(cx).candidate(cx).expect("loaded"), policy);
        window.click("acp-policy-remove-pool-a", cx);
        window.render_frame(cx);
        assert!(window.try_find("acp-policy-pool-a").is_none());
        assert!(window.find("acp-policy-pools-empty").visible());
        let current = page.read(cx).candidate(cx).expect("removed override");
        assert!(current.pools.is_empty());
        assert_eq!(current.default_agent, policy.default_agent);
        assert_eq!(current.pool_size, policy.pool_size);
        assert_eq!(current.pool_targets(None).get(&"a".into()), Some(&1));
        assert_eq!(
            page.read(cx).pool_candidates(),
            vec!["a".into(), "b".into()]
        );
        page.update(cx, |this, cx| {
            this.add_pool("b".into(), window, cx);
            let updated = this.candidate(cx).expect("add pool");
            assert_eq!(updated.pools.get(&"b".into()), Some(&1));
            this.accept(saved.clone(), window, cx)
                .expect("reload server version");
            assert_eq!(this.candidate(cx).expect("restored zero override"), policy);
        });
    })
    .expect("explicit pool list");
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
            assert!(!this.status(cx).is_empty());
            assert!(this.disabled());
        });
        entity = Some(page.clone());
        Root::new(page, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("acp-policy-save").is_none());
        let page = entity.as_ref().expect("page").read(cx);
        assert!(page.load_task.is_none());
        assert!(page.write.is_none());
        assert!(page.baseline.is_none());
    })
    .expect("disconnected state");
}
