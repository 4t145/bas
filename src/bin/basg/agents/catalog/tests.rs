use super::*;
use bas::config::agent::Command;

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
