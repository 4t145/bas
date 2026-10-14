use super::*;
use bas::authentication::{Execution, Source};
use pb::authentication_event::Kind;
use serde_json::json;

fn event(kind: Kind) -> Event {
    decode(pb::AuthenticationEvent { kind: Some(kind) })
}

fn payload(value: serde_json::Value) -> pb::AcpPayload {
    pb::AcpPayload {
        json: serde_json::to_vec(&value).expect("fixture JSON"),
    }
}

#[test]
fn start_encodes_the_exact_selected_method_and_registration() {
    let spec = AgentSpec::embedded();
    let method = Method {
        id: "account".into(),
        name: "Account".into(),
        description: Some("current description".into()),
        raw: json!({"id":"account","custom":"fresh metadata"}),
        source: Source::Initialize,
        execution: Execution::Unsupported {
            reason: "fixture".into(),
        },
    };
    let input = start_input(&spec, &method).expect("start input");
    let Some(pb::authentication_input::Kind::Start(start)) = input.kind else {
        panic!("start control");
    };
    assert_eq!(start.agent, spec.id.to_string());
    assert_eq!(start.method_id, method.id);
    assert_eq!(
        serde_json::from_slice::<AgentSpec>(&start.expected_spec_json).expect("spec"),
        spec
    );
    assert_eq!(
        serde_json::from_slice::<Method>(&start.expected_method_json).expect("method"),
        method
    );
}

#[test]
fn controls_preserve_request_identity_response_values_and_terminal_exit_code() {
    let response =
        json!({"action":"accept","content":{"flag":true,"count":7},"_meta":{"id":"url"}});
    let input = encode(Control::ElicitationResponse {
        request_id: "transport-request".into(),
        response: response.clone(),
    });
    let Some(pb::authentication_input::Kind::ElicitationResponse(answer)) = input.kind else {
        panic!("elicitation response");
    };
    assert_eq!(answer.request_id, "transport-request");
    assert_eq!(
        answer
            .response
            .expect("response")
            .decode::<serde_json::Value>()
            .expect("JSON"),
        response
    );
    assert!(matches!(
        encode(Control::TerminalResult { exit_code: 23 }).kind,
        Some(pb::authentication_input::Kind::TerminalResult(
            pb::TerminalAuthenticationResult { exit_code: 23 }
        ))
    ));
}

#[test]
fn elicitation_decode_preserves_transport_identity_and_domain_request() {
    let raw = json!({"requestId":"domain-request","mode":"url","message":"Sign in","url":"https://example.com/login","elicitationId":"url-1"});
    let Event::Elicitation {
        request_id,
        request,
    } = event(Kind::Elicitation(pb::PendingElicitation {
        request_id: "transport-request".into(),
        request: Some(payload(raw.clone())),
        answered: true,
    }))
    else {
        panic!("elicitation event");
    };
    assert_eq!(request_id, "transport-request");
    assert_eq!(request.expect("domain request").raw, raw);
    let completion = json!({"elicitationId":"url-1","_meta":{"correlation":"kept"}});
    let Event::ElicitationComplete(Some(value)) =
        event(Kind::ElicitationComplete(payload(completion.clone())))
    else {
        panic!("completion event");
    };
    assert_eq!(value, completion);
}

#[test]
fn invalid_payloads_keep_their_individual_event_semantics() {
    assert!(matches!(
        event(Kind::Elicitation(pb::PendingElicitation::default())),
        Event::Elicitation {
            request: Err(Failure::MissingRequest),
            ..
        }
    ));
    for request in [
        payload(json!({})),
        pb::AcpPayload {
            json: b"{".to_vec(),
        },
    ] {
        assert!(matches!(
            event(Kind::Elicitation(pb::PendingElicitation {
                request: Some(request),
                ..Default::default()
            })),
            Event::Elicitation {
                request: Err(Failure::InvalidPayload(_)),
                ..
            }
        ));
    }
    assert!(matches!(
        event(Kind::ElicitationComplete(pb::AcpPayload {
            json: b"{".to_vec()
        })),
        Event::ElicitationComplete(None)
    ));
    assert!(matches!(
        event(Kind::Terminal(payload(json!({})))),
        Event::Terminal(Err(Failure::InvalidPayload(_)))
    ));
    assert!(matches!(
        decode(pb::AuthenticationEvent::default()),
        Event::Unknown
    ));
}

#[test]
fn terminal_decodes_the_exact_invocation_without_launching_it() {
    let plan = Invocation {
        program: "/fixture/agent".into(),
        args: vec!["login".into(), "--account".into()],
        env: [("ACCOUNT".into(), "fixture".into())].into(),
    };
    let Event::Terminal(Ok(decoded)) = event(Kind::Terminal(payload(
        serde_json::to_value(&plan).expect("invocation"),
    ))) else {
        panic!("terminal plan");
    };
    assert_eq!(decoded.program, plan.program);
    assert_eq!(decoded.args, plan.args);
    assert_eq!(decoded.env, plan.env);
}

#[test]
fn final_check_is_distinct_from_eof_even_when_check_payload_is_invalid() {
    let decoded = next_event(Ok(Some(pb::AuthenticationEvent {
        kind: Some(Kind::Check(pb::CheckAgentResponse {
            status: pb::AgentCheckStatus::AuthenticationRequired.into(),
            ..Default::default()
        })),
    })));
    assert!(matches!(decoded, Ok(Event::Check(Status::Failed(_)))));
    assert!(matches!(next_event(Ok(None)), Err(Failure::StreamEnded)));
    assert!(matches!(
        event(Kind::Check(pb::CheckAgentResponse {
            status: pb::AgentCheckStatus::Connected.into(),
            resolved_program: "/fixture/agent".into(),
            ..Default::default()
        })),
        Event::Check(Status::Connected(program)) if program == "/fixture/agent"
    ));
}

#[test]
fn rpc_failure_preserves_code_details_and_existing_presentation() {
    let status = tonic::Status::with_details(
        tonic::Code::Unavailable,
        "  disconnected  ",
        b"diagnostic details".as_slice().into(),
    );
    let diagnostic = status.to_string();
    let Err(Failure::StreamRpc(error)) = next_event(Err(status)) else {
        panic!("stream failure");
    };
    assert!(matches!(
        &error,
        crate::connection::Failure::Rpc { code, message, details, diagnostic: actual }
        if *code == tonic::Code::Unavailable && message == "  disconnected  "
            && details == b"diagnostic details" && actual == &diagnostic
    ));
    assert_eq!(
        Failure::StreamRpc(error).presentation(),
        format!("{}：disconnected", crate::i18n::tr!("auth-failed"))
    );
    for error in [
        Failure::StartRpc(tonic::Status::unimplemented("").into()),
        Failure::StreamRpc(tonic::Status::unimplemented("").into()),
    ] {
        assert_eq!(error.presentation(), crate::i18n::tr!("auth-upgrade"));
    }
}
