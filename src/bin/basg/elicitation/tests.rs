use super::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, component::Root, px, size};

fn request(mode: &str) -> Request {
    let mut raw = if mode == "form" {
        json!({"mode":"form","message":"Configure account","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2},"age":{"type":"integer","minimum":1},"enabled":{"type":"boolean","default":true}},"required":["name","age","enabled"]}})
    } else if mode == "choices" {
        json!({
            "mode": "form",
            "message": "Choose account options",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "region": {
                        "type": "string",
                        "oneOf": [
                            {"const": "eu", "title": "Europe"},
                            {"const": "ap", "title": "Asia Pacific"}
                        ]
                    },
                    "permissions": {
                        "type": "array",
                        "minItems": 1,
                        "uniqueItems": true,
                        "items": {"enum": ["read", "write"]}
                    }
                },
                "required": ["region", "permissions"]
            }
        })
    } else {
        json!({"mode":"url","message":"Authorize account","url":"https://example.test/authorize?scope=profile","elicitationId":"url-1"})
    };
    raw.as_object_mut()
        .expect("request object")
        .insert("requestId".into(), json!("auth-1"));
    bas::authentication::elicitation::parse_request(&raw).expect("valid request")
}

#[gpui_kit::test]
fn labeled_checkbox_renders_small_typography_independent_of_control_size(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};

    struct Probe(Rc<RefCell<Option<gpui_kit::TextStyle>>>);
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let metrics = self.0.clone();
            crate::ui::checkbox("checkbox-typography")
                .label("Read")
                .child(
                    gpui_kit::canvas(
                        move |_, window, _| {
                            *metrics.borrow_mut() = Some(window.text_style());
                        },
                        |_, _, _, _| {},
                    )
                    .size(gpui_kit::px(1.)),
                )
        }
    }

    cx.update(gpui_kit::init);
    let metrics = Rc::new(RefCell::new(None));
    let window = cx.open_window(size(px(400.), px(200.)), |_, _| Probe(metrics.clone()));
    for rem_size in [16., 22.] {
        cx.update_window(window.into(), |_, window, cx| {
            window.set_rem_size(px(rem_size));
            window.render_frame(cx);
            let metrics = metrics.borrow();
            let actual = metrics.as_ref().expect("checkbox content was prepainted");
            assert_eq!(
                actual.font_size.to_pixels(px(rem_size)),
                crate::ui::SMALL.to_pixels(px(rem_size))
            );
            assert_eq!(window.find("checkbox-typography").label(), Some("Read"));
        })
        .expect("render checkbox typography");
    }
}

#[gpui_kit::test]
fn choice_fields_validate_required_values_and_toggle_native_selections(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut panel = None;
    let window = cx.open_window(size(px(720.), px(500.)), |window, cx| {
        let entity = cx.new(|cx| Panel::new("choice-1".into(), request("choices"), window, cx));
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
        assert!(events.borrow().is_empty());
        assert!(panel.read(cx).errors.contains_key("region"));
        assert!(panel.read(cx).errors.contains_key("permissions"));
        window.render_frame(cx);
        window.click("elicitation-input-region-1", cx);
        window.render_frame(cx);
        window.click("elicitation-input-permissions-0", cx);
        window.render_frame(cx);
        window.click("elicitation-input-permissions-1", cx);
        window.render_frame(cx);
        window.click("elicitation-input-permissions-0", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert!(panel.read(cx).submitting);
        assert!(panel.read(cx).errors.is_empty());
    })
    .expect("choice form");
    assert_eq!(
        *events.borrow(),
        vec![json!({"action":"accept","content":{"region":"ap","permissions":["write"]}})]
    );
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
        assert!(panel.message.contains("在浏览器中完成操作"));
    })
    .expect("URL acceptance is pending");
}
