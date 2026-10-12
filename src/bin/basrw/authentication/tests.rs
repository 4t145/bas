use super::*;
use basr::authentication::{Arguments, Source, Terminal};
use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt};
use serde_json::json;

#[test]
fn authentication_requires_the_current_checksum_bound_registration() {
    use basr::config::{AgentRegistration, InstallationRef, ManagedAgent, agent::Command};
    let directory = tempfile::tempdir().expect("installation outside settings profile");
    let path = directory.path().join(basr::config::INSTALLATION_FILE);
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
            basr::config::agent::Command::new("/fixture/agent", []),
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
        assert_eq!(panel.message, "请重新打开 BASR 以启用认证。");
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
                result.kind,
                Some(pb::authentication_input::Kind::TerminalResult(
                    pb::TerminalAuthenticationResult { exit_code: 23 }
                ))
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
                panel.event(Ok(pb::AuthenticationEvent { kind:Some(pb::authentication_event::Kind::Elicitation(pb::PendingElicitation { request_id:id.into(),request:Some(pb::AcpPayload {json:serde_json::to_vec(&raw).expect("JSON")}),answered:false })) }),window,cx);
            }
            panel.event(Ok(pb::AuthenticationEvent { kind:Some(pb::authentication_event::Kind::ElicitationCancelled("unknown".into())) }),window,cx);
            assert_eq!(panel.forms.len(),2);
            for _ in 0..2 { panel.event(Ok(pb::AuthenticationEvent { kind:Some(pb::authentication_event::Kind::ElicitationCancelled("first".into())) }),window,cx); }
            assert_eq!(panel.forms.keys().cloned().collect::<Vec<_>>(),vec!["second"]);
            panel.event(Err("authentication stream closed".into()),window,cx);
            assert!(panel.forms.is_empty()); assert!(panel.input.is_none()); assert!(matches!(panel.status,Status::Failed(_)));
        });
    }).expect("request cancellation and failed flow");
}
