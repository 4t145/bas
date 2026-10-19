use super::*;

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
