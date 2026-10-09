use super::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, component::Root, px, size};

fn request(mode: &str) -> Request {
    let mut raw = if mode == "form" {
        json!({"mode":"form","message":"Configure account","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2},"age":{"type":"integer","minimum":1},"enabled":{"type":"boolean","default":true}},"required":["name","age","enabled"]}})
    } else {
        json!({"mode":"url","message":"Authorize account","url":"https://example.test/authorize?scope=profile","elicitationId":"url-1"})
    };
    raw.as_object_mut()
        .expect("request object")
        .insert("requestId".into(), json!("auth-1"));
    basr::authentication::elicitation::parse_request(&raw).expect("valid request")
}

#[gpui_kit::test]
fn form_keeps_errors_local_and_submits_valid_typed_content_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut panel = None;
    let window = cx.open_window(size(px(720.), px(600.)), |window, cx| {
        let entity = cx.new(|cx| Panel::new("form-1".into(), request("form"), window, cx));
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let panel = panel.expect("panel");
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let received = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&panel, move |_, event: &Answered, _| {
            received.borrow_mut().push(event.response.clone())
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert!(!panel.read(cx).submitting);
        assert!(events.borrow().is_empty());
        panel.update(cx, |panel, cx| {
            panel.inputs["name"].update(cx, |input, cx| input.set_value("Ada", window, cx));
            panel.inputs["age"].update(cx, |input, cx| input.set_value("42", window, cx));
        });
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert!(panel.read(cx).submitting);
    })
    .expect("submit form");
    assert_eq!(
        *events.borrow(),
        vec![json!({"action":"accept","content":{"name":"Ada","age":42,"enabled":true}})]
    );
}

#[gpui_kit::test]
fn accepting_url_waits_for_matching_completion_and_sends_no_form_content(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut panel = None;
    let window = cx.open_window(size(px(720.), px(400.)), |window, cx| {
        let entity = cx.new(|cx| Panel::new("url-request".into(), request("url"), window, cx));
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let panel = panel.expect("panel");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("elicitation-url").visible());
        panel.update(cx, |panel, cx| panel.answer("accept", cx));
        let panel = panel.read(cx);
        assert!(panel.submitting && panel.accepted);
        assert!(!panel.completed_by(&json!({"elicitationId":"other"})));
        assert!(panel.completed_by(&json!({"elicitationId":"url-1"})));
        assert!(panel.message.contains("等待 Agent"));
    })
    .expect("URL acceptance is pending");
}
