use super::*;
use serde_json::json;

#[test]
fn patches_preserve_omitted_fields_clear_null_and_replace_state() {
    let mut graph = Subagents::default();
    let patch = |value| serde_json::from_value::<acp::SubagentUpdate>(value).expect("patch");
    graph.apply(None, &patch(json!({"sessionId":"child","title":"Research","state":{"state":"running"},"capabilities":{"cancel":{}}}))).expect("announce");
    graph
        .apply(
            None,
            &patch(json!({"sessionId":"child","description":"Inspect source"})),
        )
        .expect("patch");
    assert!(graph.iter().next().expect("child").is_active());
    graph
        .apply(
            None,
            &patch(json!({"sessionId":"child","title":null,"state":null,"capabilities":null})),
        )
        .expect("clear");
    let child = graph.iter().next().expect("child");
    assert!(child.title.is_none() && child.state.is_none() && child.capabilities.is_none());
    assert_eq!(child.description.as_deref(), Some("Inspect source"));
    graph
        .apply(
            None,
            &patch(json!({"sessionId":"child","state":{"state":"requires_action"}})),
        )
        .expect("waiting");
    assert!(graph.iter().next().expect("child").is_active());
    graph
        .apply(
            None,
            &patch(json!({"sessionId":"child","state":{"state":"idle","stopReason":"end_turn"}})),
        )
        .expect("complete");
    assert!(!graph.iter().next().expect("child").is_active());
}

#[test]
fn associations_require_known_parents_and_cannot_change_ownership() {
    let mut graph = Subagents::default();
    assert!(
        graph
            .apply(Some("unknown"), &acp::SubagentUpdate::new("child"))
            .is_err()
    );
    assert!(graph.is_empty());
    graph
        .apply(None, &acp::SubagentUpdate::new("child"))
        .expect("child");
    graph
        .apply(Some("child"), &acp::SubagentUpdate::new("nested"))
        .expect("nested");
    assert!(
        graph
            .apply(None, &acp::SubagentUpdate::new("nested"))
            .is_err()
    );
    assert!(
        graph
            .apply(Some("nested"), &acp::SubagentUpdate::new("child"))
            .is_err()
    );
}
