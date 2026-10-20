use super::*;
use bas::config::agent::Command;

#[test]
fn local_file_authority_requires_ipc_even_for_loopback_tcp() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("fixture runtime");
    for (value, eligible) in [
        ("local", true),
        ("unix:///tmp/bas.sock", true),
        ("pipe://bas-test", true),
        ("tcp://127.0.0.1", false),
        ("[::1]", false),
        ("tcp://192.0.2.1", false),
    ] {
        let service = Service::new(
            value.parse().expect("valid target"),
            runtime.handle().clone(),
        );
        assert_eq!(service.local(), eligible, "{value}");
    }
}

fn operation(complete: bool, spec_json: Vec<u8>) -> pb::AgentOperation {
    pb::AgentOperation {
        phase: "ignored transport phase".into(),
        message: "installation progress".into(),
        complete,
        spec_json,
    }
}

#[test]
fn completed_progress_carries_the_decoded_spec() {
    let spec = AgentSpec::acp("fixture", Command::new("/fixture/agent", []));
    let progress = decode(operation(
        true,
        serde_json::to_vec(&spec).expect("fixture spec"),
    ));
    assert!(progress.complete);
    assert_eq!(progress.message, "installation progress");
    assert_eq!(progress.spec, Some(spec));
}
