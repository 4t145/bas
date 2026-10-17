use super::*;
use crate::settings::agents::Check;

fn exercise(fail: bool, cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    let fixture = Fixture::new(path.clone());
    let spec = fixture.complete();
    let before = std::fs::read(&path).expect("settings before logout");
    if fail {
        fixture.probes.lock().expect("probes").logout_error = Some(RpcStatus::failed_precondition(
            "Agent does not advertise logout",
        ));
    }
    let (window, page) = fixture.open(cx);
    cx.update(|cx| {
        cx.set_global(crate::agents::Changes::default());
        page.update(cx, |page, _| {
            page.checks.insert(
                spec.id.clone(),
                Check {
                    spec: spec.clone(),
                    status: crate::agents::Status::Connected("/fixture/codex-acp".into()),
                    completed_at: Some(Instant::now()),
                    _task: None,
                },
            );
        });
    });
    let previous = cx.update(|cx| cx.windows());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-logout-codex", cx);
    })
    .expect("request logout");
    let confirmation = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !previous.contains(handle))
            .expect("confirmation")
    });
    fixture
        .runtime
        .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
    assert!(
        fixture.probes.lock().expect("probes").logouts.is_empty(),
        "confirmation precedes the RPC"
    );
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-close", cx);
    })
    .expect("cancel");
    assert!(fixture.probes.lock().expect("probes").logouts.is_empty());
    let previous = cx.update(|cx| cx.windows());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-logout-codex", cx);
    })
    .expect("request again");
    let confirmation = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !previous.contains(handle))
            .expect("confirmation")
    });
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm");
    fixture.pump_until(cx, |cx| !page.read(cx).logging_out(&spec));
    {
        let probes = fixture.probes.lock().expect("probes");
        assert_eq!(probes.logouts.len(), 1);
        assert_eq!(probes.logouts[0].agent, "codex");
        let expected: bas::config::AgentSpec =
            serde_json::from_slice(&probes.logouts[0].expected_spec_json).expect("spec");
        assert_eq!(expected, spec);
        assert!(
            probes.checks.is_empty(),
            "logout must not probe session/new afterwards"
        );
        assert!(probes.logins.is_empty());
        assert!(probes.unrelated.is_empty());
    }
    cx.update(|cx| {
        assert!(!page.read(cx).checks.contains_key(&spec.id));
        let message = page.read(cx).logout_message(&spec).expect("feedback");
        assert!(
            message.contains(if fail { "未完成" } else { "已确认" }),
            "{message}"
        );
    });
    assert_eq!(std::fs::read(path).expect("settings after logout"), before);
}

#[gpui_kit::test]
fn logout_requires_confirmation_preserves_settings_and_clears_readiness(cx: &mut TestAppContext) {
    exercise(false, cx);
}

#[gpui_kit::test]
fn logout_failure_is_visible_and_does_not_retry_or_reauthenticate(cx: &mut TestAppContext) {
    exercise(true, cx);
}
