use super::*;
use serde_json::json;

pub(super) fn pending(
    session: SessionId,
    mode: &str,
    answered: bool,
) -> connection::PendingElicitation {
    let mut request = if mode == "form" {
        json!({"mode":"form","message":"Name this task","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2}},"required":["name"]}})
    } else if mode == "choices" {
        json!({
            "mode": "form",
            "message": "选择部署选项",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "channel": {
                        "type": "string",
                        "title": "环境",
                        "oneOf": [
                            {"const": "dev", "title": "开发"},
                            {"const": "prod", "title": "生产"}
                        ]
                    },
                    "features": {
                        "type": "array",
                        "title": "检查",
                        "minItems": 1,
                        "uniqueItems": true,
                        "items": {
                            "anyOf": [
                                {"const": "lint", "title": "代码检查"},
                                {"const": "test", "title": "单元测试"}
                            ]
                        }
                    }
                },
                "required": ["channel", "features"]
            }
        })
    } else {
        json!({"mode":"url","message":"Authorize","url":"https://example.test/authorize","elicitationId":"url-1"})
    };
    request
        .as_object_mut()
        .expect("request")
        .insert("sessionId".into(), json!(session.to_string()));
    connection::PendingElicitation {
        request_id: "request-1".into(),
        request: Ok(
            bas::authentication::elicitation::parse_request(&request).expect("valid elicitation")
        ),
        answered,
    }
}

#[gpui_kit::test]
fn compact_choices_show_waiting_status_and_submit_native_values(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.toggle_mode(window, cx);
            assert!(!view.window.output_open);
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch,
                    pending: vec![pending(meta.id, "choices", false)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.window.output_open);
            assert_eq!(view.stream_label(), "等待你的回答…");
            assert_eq!(view.activity_label().as_deref(), Some("等待你的回答…"));
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        assert!(window.find("thread-elicitations").visible());
        for choice in [
            "elicitation-input-channel-0",
            "elicitation-input-channel-1",
            "elicitation-input-features-0",
            "elicitation-input-features-1",
            "elicitation-submit",
        ] {
            assert!(
                window.find(choice).visible(),
                "{choice} stays visible in compact mode"
            );
        }
        let panel = window.find("thread-elicitations").bounds();
        assert!(panel.bottom() <= window.find("composer-editor").bounds().top());
        window.click("elicitation-input-channel-1", cx);
        window.render_frame(cx);
        window.click("elicitation-input-features-0", cx);
        window.render_frame(cx);
        window.click("elicitation-input-features-1", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert!(
            fixture.view.read(cx).selected.elicitations["request-1"]
                .entity
                .read(cx)
                .submitting
        );
    })
    .expect("compact elicitation choices");
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::RespondElicitation { session, request_id, response })
            if session == meta.id
                && request_id == "request-1"
                && response == json!({"action":"accept","content":{"channel":"prod","features":["lint","test"]}})
    ));
    assert!(
        fixture.commands.try_recv().is_err(),
        "one response per request"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch,
                    pending: Vec::new(),
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert_eq!(view.stream_label(), "已同步");
            assert!(view.activity_label().is_none());
        });
        window.render_frame(cx);
        assert!(window.try_find("thread-elicitations").is_none());
    })
    .expect("completed elicitation restores idle status");
}

#[gpui_kit::test]
fn wide_elicitation_panel_stays_within_the_content_column(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.expanded = false;
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch,
                    pending: vec![pending(meta.id, "form", false)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
        });
        window.resize(size(px(2000.), px(1000.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        let panel = window.find("elicitation-request-1").bounds();
        let content = window.find("thread-body").bounds();
        assert!(
            content.size.width > px(chat::CONTENT_WIDTH),
            "wide viewport leaves room around the content column"
        );
        assert_eq!(
            panel.size.width,
            px(chat::CONTENT_WIDTH),
            "elicitation panel matches the content column"
        );
        assert!(
            panel.left() - content.left() > px(0.) && content.right() - panel.right() > px(0.),
            "elicitation panel {panel:?} keeps side margins"
        );
    })
    .expect("wide elicitation width");
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
                .input
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch,
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
            fixture.view.read(cx).composer.input.read(cx).value(),
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
            assert!(view.selected.elicitations.is_empty());
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch - 1,
                    pending: vec![pending(meta.id, "form", false)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.selected.elicitations.is_empty());
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
                    epoch: view.selected.epoch,
                    pending: vec![pending(meta.id, "url", true)],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(
                view.selected.elicitations["request-1"]
                    .entity
                    .read(cx)
                    .submitting
            );
        });
        window.render_frame(cx);
        assert!(window.find("elicitation-url").visible());
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Elicitations {
                    epoch: view.selected.epoch,
                    pending: vec![pending(meta.id, "url", true)],
                    completed_ids: vec!["request-1".into()],
                },
                window,
                cx,
            );
            assert!(view.selected.elicitations.is_empty());
        });
        window.render_frame(cx);
        assert!(window.try_find("elicitation-url").is_none());
    })
    .expect("URL opening never substitutes for backend completion");
}
