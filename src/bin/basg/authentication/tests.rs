use super::*;
use bas::authentication::{Arguments, Source, Terminal};
use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt};
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

#[gpui_kit::test]
fn method_clicks_keep_agent_names_and_begin_without_a_confirmation_window(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
        crate::application::init(cx);
    });
    let methods = vec![
        Method {
            id: "terminal".into(),
            name: "Moonshot Account".into(),
            description: Some("Use the account owned by this Agent".into()),
            raw: json!({"id":"terminal"}),
            source: Source::Initialize,
            execution: Execution::Terminal(Terminal {
                args: vec!["login".into()],
                env: BTreeMap::new(),
                arguments: Arguments::Append,
                command: None,
            }),
        },
        Method {
            id: "future".into(),
            name: "Experimental sign-in".into(),
            description: None,
            raw: json!({"id":"future"}),
            source: Source::Initialize,
            execution: Execution::Unsupported {
                reason: "This Agent needs a newer authentication client".into(),
            },
        },
    ];
    let mut authentication = None;
    let window = cx.open_window(size(px(720.), px(400.)), |window, cx| {
        let spec = AgentSpec::acp(
            "tool",
            bas::config::agent::Command::new("/fixture/agent", []),
        );
        let panel = cx.new(|_| {
            Authentication::new(
                spec,
                Status::AuthenticationRequired {
                    program: "/fixture/agent".into(),
                    methods,
                },
                "/fixture/settings.json".into(),
                window.window_handle(),
            )
        });
        authentication = Some(panel.clone());
        Root::new(panel, window, cx)
    });
    let authentication = authentication.expect("authentication panel");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("authentication-method-0").label(),
            Some("Moonshot Account")
        );
        assert_eq!(
            window.find("authentication-method-1").label(),
            Some("Experimental sign-in")
        );
        let state = window.find("authentication-state").bounds();
        let method = window.find("authentication-method-0").bounds();
        let description = window.find("authentication-description-0").bounds();
        assert!(
            state.bottom() < method.top(),
            "the concise status has its own row above authentication choices"
        );
        assert!(
            method.bottom() + px(4.) <= description.top(),
            "method descriptions have visible spacing below their action"
        );
        let before = cx.windows();
        window.click("authentication-method-1", cx);
        assert_eq!(cx.windows(), before);
        assert!(window.try_find("authentication-error").is_none());
        assert!(matches!(
            authentication.read(cx).status,
            Status::AuthenticationRequired { .. }
        ));
        window.click("authentication-method-0", cx);
        window.render_frame(cx);
        assert_eq!(
            cx.windows(),
            before,
            "a terminal method begins in place without opening a confirmation window"
        );
        assert!(window.find("authentication-error").visible());
        let panel = authentication.read(cx);
        assert_eq!(panel.status_label(), "认证未完成");
        assert_eq!(panel.message, "请重新打开 BasG 以启用认证。");
        assert!(matches!(panel.status, Status::Failed(_)));
    })
    .expect("method actions");
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
fn agent_cancellation_removes_only_its_request_and_failures_clear_pending_controls(
    cx: &mut TestAppContext,
) {
    let (window, panel) = empty_panel(cx);
    cx.update_window(window.into(),|_,window,cx| {
        panel.update(cx,|panel,cx| {
            for id in ["first","second"] {
                let raw = json!({"requestId":"auth-rpc","mode":"form","message":"Provide details","requestedSchema":{"type":"object","properties":{"name":{"type":"string"}}}});
                panel.event(Ok(service::Event::Elicitation { request_id: id.into(), request: Ok(bas::authentication::elicitation::parse_request(&raw).expect("form request")) }),window,cx);
            }
            panel.event(Ok(service::Event::ElicitationCancelled("unknown".into())),window,cx);
            assert_eq!(panel.forms.len(),2);
            for _ in 0..2 { panel.event(Ok(service::Event::ElicitationCancelled("first".into())),window,cx); }
            assert_eq!(panel.forms.keys().cloned().collect::<Vec<_>>(),vec!["second"]);
            panel.event(Err(service::Failure::External("authentication stream closed".into())),window,cx);
            assert!(panel.forms.is_empty()); assert!(panel.input.is_none()); assert!(matches!(panel.status,Status::Failed(_)));
        });
    }).expect("request cancellation and failed flow");
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

#[gpui_kit::test]
fn final_check_replaces_methods_and_rejects_stale_choices(cx: &mut TestAppContext) {
    let (window, panel) = empty_panel(cx);
    let method = Method {
        id: "account".into(),
        name: "Current account".into(),
        description: None,
        raw: json!({"id":"account"}),
        source: Source::Initialize,
        execution: Execution::Agent,
    };
    let (input, _receive) = mpsc::unbounded_channel();
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.input = Some(input);
            panel.event(
                Ok(service::Event::Check(Status::AuthenticationRequired {
                    program: "/fixture/agent".into(),
                    methods: vec![method.clone()],
                })),
                window,
                cx,
            );
            assert!(panel.input.is_none());
            assert_eq!(panel.methods, vec![method.clone()]);
            let mut stale = method.clone();
            stale.name = "Outdated account".into();
            let message = panel.message.clone();
            panel.begin(stale, cx);
            assert_eq!(
                panel.message, message,
                "stale method never starts a new stream"
            );
            panel.event(
                Ok(service::Event::Check(Status::Connected(
                    "/fixture/agent".into(),
                ))),
                window,
                cx,
            );
            assert_eq!(panel.message, crate::i18n::tr!("auth-complete"));
            assert!(panel.status.connected());
        });
    })
    .expect("final check");
}

#[gpui_kit::test]
fn malformed_terminal_event_cancels_without_launching_a_child(cx: &mut TestAppContext) {
    let (window, panel) = empty_panel(cx);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    cx.update(|cx| {
        crate::agents::init(
            "127.0.0.1:7337".parse().expect("address"),
            runtime.handle().clone(),
            cx,
        );
    });
    let (input, _receive) = mpsc::unbounded_channel();
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.input = Some(input);
            let generation = panel.generation;
            panel.event(
                Ok(service::Event::Terminal(Err(
                    service::Failure::InvalidPayload("invalid plan".into()),
                ))),
                window,
                cx,
            );
            assert_eq!(panel.generation, generation + 1);
            assert!(panel.input.is_none());
            assert!(panel._terminal.is_none());
            assert_eq!(panel.message, crate::i18n::tr!("auth-cancelled"));
        });
    })
    .expect("terminal cancellation");
}
