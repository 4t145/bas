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
