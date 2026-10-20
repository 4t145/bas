use super::*;
use gpui_kit::{TestAppContext, component::Root, px, size};
use serde_json::json;

fn empty_panel(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<Root>, Entity<Authentication>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut panel = None;
    let window = cx.open_window(size(px(720.), px(500.)), |window, cx| {
        let entity = cx.new(|_| {
            Authentication::new(
                AgentSpec::embedded(),
                Status::AuthenticationRequired {
                    program: String::new(),
                    methods: Vec::new(),
                },
                "/fixture/settings.json".into(),
                window.window_handle(),
                None,
            )
        });
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (window, panel.expect("panel"))
}

#[gpui_kit::test]
fn authentication_keeps_its_service_after_management_switches(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let first = bas::service::tls::ConnectionOptions {
        target: "mtls://same-server:4554".parse().expect("target"),
        tls: Some(bas::service::tls::ClientTls {
            ca: "/certs/ca.pem".into(),
            certificate: "/certs/first.pem".into(),
            private_key: "/certs/first.key".into(),
            server_name: None,
        }),
    };
    let mut second = first.clone();
    second.tls.as_mut().expect("TLS").certificate = "/certs/second.pem".into();
    second.tls.as_mut().expect("TLS").private_key = "/certs/second.key".into();
    let (window, _) = empty_panel(cx);
    cx.update_window(window.into(), |_, window, cx| {
        check::init(first.clone(), runtime.handle().clone(), cx);
        let panel = Authentication::new(
            AgentSpec::embedded(),
            Status::Installed(String::new()),
            "/fixture/settings.json".into(),
            window.window_handle(),
            cx.try_global::<check::Service>().cloned(),
        );
        check::init(second.clone(), runtime.handle().clone(), cx);
        assert_eq!(
            panel.service.as_ref().expect("pinned service").address(),
            first
        );
        assert_eq!(cx.global::<check::Service>().address(), second);
    })
    .expect("authentication window");
}

#[gpui_kit::test]
fn cancelled_terminal_attempt_cannot_answer_a_later_authentication_stream(cx: &mut TestAppContext) {
    let (window, panel) = empty_panel(cx);
    let (input, mut receive) = mpsc::unbounded_channel();
    cx.update_window(window.into(), |_, _, cx| {
        panel.update(cx, |panel, cx| {
            let old = panel.generation;
            panel.cancel(cx);
            panel.input = Some(input);
            panel.terminal_completed(old, Ok(0), cx);
            assert!(receive.try_recv().is_err());
            panel.terminal_completed(panel.generation, Ok(23), cx);
            let result = receive.try_recv().expect("current completion");
            assert!(matches!(
                result,
                service::Control::TerminalResult { exit_code: 23 }
            ));
        });
    })
    .expect("terminal attempt guard");
}

#[gpui_kit::test]
fn invalid_events_preserve_the_active_attempt_and_url_completion_identity(cx: &mut TestAppContext) {
    let (window, panel) = empty_panel(cx);
    let (input, _receive) = mpsc::unbounded_channel();
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.input = Some(input);
            panel.status = Status::Authenticating;
            for id in ["first", "second"] {
                let raw = json!({
                    "mode":"url", "message":"Sign in", "url":"https://example.com/login",
                    "elicitationId":id, "requestId":"authentication"
                });
                panel.event(
                    Ok(service::Event::Elicitation {
                        request_id: format!("request-{id}"),
                        request: Ok(bas::authentication::elicitation::parse_request(&raw)
                            .expect("URL request")),
                    }),
                    window,
                    cx,
                );
            }
            panel.event(
                Ok(service::Event::Elicitation {
                    request_id: "invalid".into(),
                    request: Err(service::Failure::MissingRequest),
                }),
                window,
                cx,
            );
            assert_eq!(panel.forms.len(), 2);
            assert!(panel.input.is_some());
            assert_eq!(
                panel.message,
                crate::i18n::tr!(
                    "elicitation-display-error",
                    error = crate::i18n::tr!("auth-empty-request")
                )
            );
            let message = panel.message.clone();
            panel.event(Ok(service::Event::ElicitationComplete(None)), window, cx);
            assert_eq!(panel.message, message);
            panel.event(Ok(service::Event::Unknown), window, cx);
            assert_eq!(panel.message, crate::i18n::tr!("auth-invalid-event"));
            assert!(panel.input.is_some());
            panel.event(
                Ok(service::Event::ElicitationComplete(Some(
                    json!({"elicitationId":"unrelated"}),
                ))),
                window,
                cx,
            );
            assert_eq!(panel.forms.len(), 2);
            assert_eq!(panel.message, crate::i18n::tr!("auth-verifying"));
            panel.event(
                Ok(service::Event::ElicitationComplete(Some(
                    json!({"elicitationId":"first"}),
                ))),
                window,
                cx,
            );
            assert_eq!(
                panel.forms.keys().cloned().collect::<Vec<_>>(),
                vec!["request-second"]
            );
            assert!(matches!(panel.status, Status::Authenticating));
        });
    })
    .expect("per-event behavior");
}
