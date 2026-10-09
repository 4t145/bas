use super::*;
use basr::service::pb;
use serde_json::json;

fn pending(session: SessionId, mode: &str, answered: bool) -> pb::PendingElicitation {
    let mut request = if mode == "form" {
        json!({"mode":"form","message":"Name this task","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2}},"required":["name"]}})
    } else {
        json!({"mode":"url","message":"Authorize","url":"https://example.test/authorize","elicitationId":"url-1"})
    };
    request
        .as_object_mut()
        .expect("request")
        .insert("sessionId".into(), json!(session.to_string()));
    pb::PendingElicitation {
        request_id: "request-1".into(),
        request: Some(pb::AcpPayload {
            json: serde_json::to_vec(&request).expect("JSON"),
        }),
        answered,
    }
}

#[gpui_kit::test]
fn session_form_preserves_composition_and_targets_only_the_current_request(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            view.update(
                Update::Elicitations {
                    epoch: view.epoch,
                    pending: vec![pending(meta.id, "form", false)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        window.click("elicitation-input-name", cx);
        window.input("Ada", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "keep draft"
        );
    })
    .expect("in-place form");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::RespondElicitation { session, request_id, response }) if session == meta.id && request_id == "request-1" && response == json!({"action":"accept","content":{"name":"Ada"}}))
    );
    let next = session();
    attach(&fixture, next.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(view.elicitation_panels.is_empty());
            view.update(
                Update::Elicitations {
                    epoch: view.epoch - 1,
                    pending: vec![pending(meta.id, "form", false)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.elicitation_panels.is_empty());
        });
        window.render_frame(cx);
        assert!(window.try_find("thread-elicitations").is_none());
    })
    .expect("stale session cannot restore the form");
}

#[gpui_kit::test]
fn accepted_session_url_stays_visible_until_authoritative_completion(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Elicitations {
                    epoch: view.epoch,
                    pending: vec![pending(meta.id, "url", true)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.elicitation_panels["request-1"].read(cx).submitting);
        });
        window.render_frame(cx);
        assert!(window.find("elicitation-url").visible());
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Elicitations {
                    epoch: view.epoch,
                    pending: vec![pending(meta.id, "url", true)],
                    completed_ids: vec!["request-1".into()],
                },
                window,
                cx,
            );
            assert!(view.elicitation_panels.is_empty());
        });
        window.render_frame(cx);
        assert!(window.try_find("elicitation-url").is_none());
    })
    .expect("URL opening never substitutes for backend completion");
}
