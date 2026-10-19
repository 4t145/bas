use super::*;
use gpui_kit::{TestAppContext, component::Root, px, size};
use serde_json::json;

#[test]
fn authentication_requires_the_current_checksum_bound_registration() {
    use bas::config::{AgentRegistration, InstallationRef, ManagedAgent, agent::Command};
    let directory = tempfile::tempdir().expect("installation outside settings profile");
    let path = directory.path().join(bas::config::INSTALLATION_FILE);
    let spec = AgentSpec::acp("managed", Command::new("/managed/acp", []));
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "spec": spec,
    }))
    .expect("descriptor");
    std::fs::write(&path, &bytes).expect("write descriptor");
    let managed = AgentRegistration::Managed(ManagedAgent {
        id: spec.id.clone(),
        installation: InstallationRef::from_bytes(path.clone(), &bytes).expect("reference"),
    });
    assert!(service::matches_registration(
        std::slice::from_ref(&managed),
        &spec
    ));
    assert!(
        !service::matches_registration(&[managed.clone(), spec.clone().into()], &spec),
        "ambiguous membership fails closed"
    );
    std::fs::write(&path, [bytes.as_slice(), b"\n"].concat()).expect("change descriptor bytes");
    assert!(
        !service::matches_registration(std::slice::from_ref(&managed), &spec),
        "even an equivalent launch config needs the registered descriptor hash"
    );
    std::fs::remove_file(&path).expect("remove descriptor");
    assert!(!service::matches_registration(
        std::slice::from_ref(&managed),
        &spec
    ));
    std::fs::write(&path, &bytes).expect("restore descriptor");
    let wrong_identity = AgentRegistration::Managed(ManagedAgent {
        id: "another-agent".into(),
        installation: InstallationRef::from_bytes(path, &bytes).expect("reference"),
    });
    let mut wrong_spec = spec.clone();
    wrong_spec.id = "another-agent".into();
    assert!(!service::matches_registration(
        &[wrong_identity],
        &wrong_spec
    ));
    assert!(service::matches_registration(&[spec.clone().into()], &spec));
}

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
            )
        });
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    (window, panel.expect("panel"))
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
