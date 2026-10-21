use super::*;
use crate::model::SessionId;
use serde_json::json;

fn event(cursor: u64, value: serde_json::Value) -> Event {
    Event::new(
        SessionId::new(),
        Cursor::from_raw(cursor),
        EventKind::Acp(serde_json::from_value(value).expect("valid v1 update")),
    )
}

fn chunk(cursor: u64, id: Option<&str>) -> Event {
    event(
        cursor,
        json!({"sessionUpdate":"agent_message_chunk","messageId":id,
        "content":{"type":"text","text":"original"}}),
    )
}

#[test]
fn idless_adjacency_ignores_nonvisible_updates_and_existing_tools() {
    let mut index = DependencyIndex::default();
    let tool = event(
        1,
        json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"tool"}),
    );
    index.observe(&tool);
    index.observe(&chunk(2, None));
    index.observe(&event(3, json!({"sessionUpdate":"plan","entries":[]})));
    index.observe(&event(
        4,
        json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed"}),
    ));
    assert_eq!(index.observe(&chunk(5, None)).floor.raw(), 2);
    index.observe(&event(
        6,
        json!({"sessionUpdate":"tool_call","toolCallId":"new","title":"new"}),
    ));
    assert_eq!(index.observe(&chunk(7, None)).floor.raw(), 7);
    index.observe(&chunk(8, Some("identified")));
    assert_eq!(index.observe(&chunk(9, None)).floor.raw(), 9);
}

#[test]
fn identities_cross_turns_but_roles_are_separate() {
    let mut index = DependencyIndex::default();
    index.observe(&chunk(1, Some("id")));
    index.observe(&Event::new(
        SessionId::new(),
        Cursor::from_raw(2),
        EventKind::TurnStarted {
            turn: crate::model::TurnId::new(),
        },
    ));
    assert_eq!(index.observe(&chunk(3, Some("id"))).floor.raw(), 1);
    let user = event(
        4,
        json!({"sessionUpdate":"user_message_chunk","messageId":"id",
        "content":{"type":"text","text":"user"}}),
    );
    assert_eq!(index.observe(&user).floor.raw(), 4);
}

#[test]
fn bounded_index_and_live_partial_history_predicate() {
    let mut index = DependencyIndex::default();
    index.observe(&chunk(1, Some("known")));
    assert!(!index.may_reference_unloaded(&chunk(2, Some("known"))));
    assert!(index.may_reference_unloaded(&chunk(2, Some("unknown"))));
    assert!(!index.may_reference_unloaded(&chunk(2, None)));
    index.evict_through(Cursor::from_raw(1));
    assert!(index.anchors.is_empty());
    assert_eq!(index.observe(&chunk(2, Some("known"))).floor, Cursor::START);
}

#[test]
fn lifecycle_dependencies_preserve_completion_and_failure_suppression() {
    use crate::model::{InterruptionReason, TurnId};
    let mut index = DependencyIndex::default();
    let session = SessionId::new();
    let turn = TurnId::new();
    index.observe(&Event::new(
        session,
        Cursor::from_raw(2),
        EventKind::TurnStarted { turn },
    ));
    let finished = Event::new(
        session,
        Cursor::from_raw(5),
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        },
    );
    let dependency = index.observe(&finished);
    assert_eq!(dependency.floor.raw(), 5);
    assert_eq!(dependency.context_key.as_deref(), Some("turn"));
    assert_eq!(dependency.context_start, None);
    index.observe(&Event::new(
        session,
        Cursor::from_raw(6),
        EventKind::TurnInterrupted {
            turn,
            reason: InterruptionReason::BackendFailure {
                message: "failure".into(),
            },
        },
    ));
    let failed = Event::new(
        session,
        Cursor::from_raw(7),
        EventKind::Failed {
            message: "failure".into(),
        },
    );
    assert_eq!(index.observe(&failed).floor.raw(), 6);
    assert_eq!(
        index
            .observe(&Event {
                cursor: Cursor::from_raw(8),
                ..failed
            })
            .floor
            .raw(),
        8
    );
}

#[test]
fn live_predicate_queries_only_referenced_identities_without_mutation() {
    let mut index = DependencyIndex::default();
    for cursor in 1..1000 {
        index.observe(&chunk(cursor, Some(&format!("message-{cursor}"))));
    }
    let before = serde_json::to_vec(&index).expect("serialize");
    assert!(!index.may_reference_unloaded(&chunk(1001, Some("message-1"))));
    assert!(index.may_reference_unloaded(&event(
        1001,
        json!({
            "sessionUpdate":"tool_call_update","toolCallId":"unknown",
            "_meta":{"terminal_output":{"terminal_id":"terminal","data":"x"}}
        })
    )));
    assert_eq!(serde_json::to_vec(&index).expect("serialize"), before);
    index.observe(&event(
        1002,
        json!({"sessionUpdate":"tool_call","toolCallId":"known","title":"tool"}),
    ));
    let before = serde_json::to_vec(&index).expect("serialize");
    assert!(index.may_reference_unloaded(&event(
        1003,
        json!({
            "sessionUpdate":"tool_call_update","toolCallId":"known",
            "content":[{"type":"terminal","terminalId":"unknown-terminal"}]
        })
    )));
    assert_eq!(serde_json::to_vec(&index).expect("serialize"), before);
}
