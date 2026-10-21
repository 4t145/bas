use super::*;
use bas::model::{EventKind, Owner};
use serde_json::json;
use std::num::NonZeroUsize;

fn state() -> SessionMeta {
    SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current())
}

fn update(value: serde_json::Value) -> EventKind {
    EventKind::Acp(serde_json::from_value(value).expect("valid v1 update"))
}

fn chunk(id: Option<&str>) -> EventKind {
    update(
        json!({"sessionUpdate":"agent_message_chunk", "messageId":id,
        "content":{"type":"text","text":"original"}}),
    )
}

async fn append(
    storage: &dyn Storage,
    meta: &mut SessionMeta,
    kinds: Vec<EventKind>,
) -> Vec<Event> {
    let events: Vec<_> = kinds
        .into_iter()
        .map(|kind| {
            let event = Event::new(meta.id, meta.revision.next(), kind);
            meta.apply_event(&event);
            event
        })
        .collect();
    storage
        .commit(meta.clone(), &events, None)
        .await
        .expect("commit");
    events
}

fn independent() -> EventKind {
    EventKind::TitleChanged {
        title: "independent".into(),
    }
}

async fn window_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let empty = storage.window(meta.id, None, 1).await.expect("empty");
    assert!(empty.events.is_empty());
    assert_eq!(empty.before, None);
    assert!(matches!(
        storage.window(meta.id, None, 0).await,
        Err(Error::EmptyPage)
    ));
    let events = append(
        storage,
        &mut meta,
        vec![
            independent(),
            chunk(Some("cross-turn")),
            EventKind::TurnStarted {
                turn: TurnId::new(),
            },
            independent(),
            chunk(Some("cross-turn")),
            independent(),
        ],
    )
    .await;
    let latest = storage
        .window(meta.id, None, 1)
        .await
        .expect("independent suffix");
    assert_eq!(latest.events, events[5..]);
    assert_eq!(latest.before, Some(events[5].cursor));
    let older = storage
        .window(meta.id, latest.before, 1)
        .await
        .expect("dependency expansion");
    assert_eq!(older.events, events[1..5]);
    assert_eq!(older.before, Some(events[1].cursor));
    assert_eq!(older.meta, meta);
    assert_eq!(older.cursor, meta.revision);
    let oldest = storage
        .window(meta.id, older.before, 1)
        .await
        .expect("oldest");
    assert_eq!(oldest.events, events[..1]);
    assert_eq!(oldest.before, None);
    assert_eq!(
        storage
            .window(meta.id, Some(meta.revision.next()), 1)
            .await
            .expect("exclusive upper boundary")
            .events,
        events[5..]
    );
    assert!(matches!(
        storage
            .window(meta.id, Some(meta.revision.next().next()), 1)
            .await,
        Err(Error::Future { .. })
    ));
    for before in [Cursor::START, Cursor::from_raw(1)] {
        let empty = storage
            .window(meta.id, Some(before), 1)
            .await
            .expect("before first");
        assert!(empty.events.is_empty());
        assert!(empty.context.is_empty());
    }
}

#[tokio::test]
async fn memory_window_contract() {
    window_contract(&Memory::new(NonZeroUsize::new(64).expect("capacity"))).await;
}

#[tokio::test]
async fn sqlite_window_contract() {
    window_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

async fn presentation_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let events = append(storage, &mut meta, vec![
        update(json!({"sessionUpdate":"plan_update","plan":{"type":"markdown","planId":"p","content":"original"}})),
        update(json!({"sessionUpdate":"usage_update","used":5,"size":100})),
        update(json!({"sessionUpdate":"tool_call","toolCallId":"producer","title":"terminal",
            "_meta":{"terminal_info":{"terminal_id":"t","command":"echo original"}}})),
        update(json!({"sessionUpdate":"tool_call_update","toolCallId":"producer",
            "_meta":{"terminal_output":{"terminal_id":"t","data":"original"}}})),
        independent(),
        update(json!({"sessionUpdate":"tool_call","toolCallId":"consumer","title":"show",
            "content":[{"type":"terminal","terminalId":"t"}]})),
    ]).await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("terminal closure");
    assert_eq!(window.events, events[2..]);
    assert_eq!(window.context, events[..2]);
    let next = append(storage, &mut meta, vec![
        update(json!({"sessionUpdate":"compaction_update","compactionId":"c","status":"in_progress","summary":[{"type":"text","text":"original"}]})),
        independent(),
        update(json!({"sessionUpdate":"compaction_summary_chunk","compactionId":"c","content":{"type":"text","text":"more"}})),
    ]).await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("compaction closure");
    assert_eq!(window.events, next);
    assert_eq!(window.context, events[..2]);
    append(
        storage,
        &mut meta,
        vec![
            update(json!({"sessionUpdate":"plan_removed","planId":"p"})),
            independent(),
        ],
    )
    .await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("removed plan context");
    assert_eq!(window.context, vec![events[1].clone()]);
}

#[tokio::test]
async fn memory_presentation_dependencies() {
    presentation_contract(&Memory::new(NonZeroUsize::new(64).expect("capacity"))).await;
}

#[tokio::test]
async fn sqlite_presentation_dependencies() {
    presentation_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

async fn plan_order_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let plan = |id: &str, text: &str| {
        update(json!({
            "sessionUpdate":"plan_update","plan":{"type":"markdown","planId":id,"content":text}
        }))
    };
    let events = append(
        storage,
        &mut meta,
        vec![
            plan("a", "first"),
            plan("b", "second"),
            plan("a", "replacement"),
            independent(),
        ],
    )
    .await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("ordered plans");
    assert_eq!(window.context, events[..3]);
    let window = storage
        .window(meta.id, Some(events[3].cursor), 1)
        .await
        .expect("replacement in page");
    assert_eq!(window.context, events[..2]);
    assert_eq!(window.events, events[2..3]);
    let next = append(
        storage,
        &mut meta,
        vec![
            update(json!({"sessionUpdate":"plan_removed","planId":"a"})),
            plan("a", "reinserted"),
            independent(),
        ],
    )
    .await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("new insertion generation");
    assert_eq!(window.context, vec![events[1].clone(), next[1].clone()]);
}

#[tokio::test]
async fn memory_preserves_plan_insertion_order() {
    plan_order_contract(&Memory::new(NonZeroUsize::new(64).expect("capacity"))).await;
}

#[tokio::test]
async fn sqlite_preserves_plan_insertion_order() {
    plan_order_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

async fn transitive_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let tool = |id: &str| {
        update(json!({"sessionUpdate":"tool_call_update","toolCallId":id,"title":"original"}))
    };
    let events = append(
        storage,
        &mut meta,
        vec![tool("a"), tool("b"), tool("a"), tool("b")],
    )
    .await;
    assert_eq!(
        storage
            .window(meta.id, None, 1)
            .await
            .expect("fixed-point expansion")
            .events,
        events
    );
}

#[tokio::test]
async fn dependency_closure_reaches_fixed_point() {
    transitive_contract(&Memory::new(NonZeroUsize::new(64).expect("capacity"))).await;
    transitive_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

async fn long_turn_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let turn = TurnId::new();
    let mut kinds = vec![EventKind::TurnStarted { turn }];
    kinds.extend((0..300).map(|id| {
        update(json!({
            "sessionUpdate":"tool_call","toolCallId":format!("tool-{id}"),"title":"independent"
        }))
    }));
    kinds.push(EventKind::TurnFinished {
        turn,
        result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
    });
    let events = append(storage, &mut meta, kinds).await;
    let window = storage
        .window(meta.id, None, 256)
        .await
        .expect("bounded long turn");
    assert_eq!(window.events, events[events.len() - 256..]);
    assert_eq!(window.context, events[..1]);
    append(storage, &mut meta, vec![independent()]).await;
    assert!(
        storage
            .window(meta.id, None, 1)
            .await
            .expect("finished context removed")
            .context
            .is_empty()
    );
}

#[tokio::test]
async fn long_turn_does_not_expand_to_its_start() {
    long_turn_contract(&Memory::new(NonZeroUsize::new(512).expect("capacity"))).await;
    long_turn_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

#[tokio::test]
async fn memory_eviction_is_explicit_but_independent_suffix_works() {
    let storage = Memory::new(NonZeroUsize::new(3).expect("capacity"));
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    append(
        &storage,
        &mut meta,
        vec![
            chunk(Some("old")),
            independent(),
            independent(),
            independent(),
        ],
    )
    .await;
    assert_eq!(
        storage
            .window(meta.id, None, 1)
            .await
            .expect("independent")
            .events
            .len(),
        1
    );
    append(&storage, &mut meta, vec![chunk(Some("old"))]).await;
    assert!(matches!(
        storage.window(meta.id, None, 1).await,
        Err(Error::Expired { .. })
    ));
    append(&storage, &mut meta, vec![independent()]).await;
    assert_eq!(
        storage
            .window(meta.id, None, 1)
            .await
            .expect("independent again")
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn memory_replacement_usage_does_not_require_evicted_snapshot() {
    let storage = Memory::new(NonZeroUsize::new(2).expect("capacity"));
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let usage = || update(json!({"sessionUpdate":"usage_update","used":5,"size":100}));
    append(
        &storage,
        &mut meta,
        vec![usage(), independent(), independent()],
    )
    .await;
    assert!(matches!(
        storage.window(meta.id, None, 1).await,
        Err(Error::Expired { .. })
    ));
    let events = append(&storage, &mut meta, vec![usage()]).await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("replacement snapshot");
    assert_eq!(window.events, events);
    assert!(window.context.is_empty());
}

#[tokio::test]
async fn sqlite_reuses_index_across_reopen_and_builds_missing_index_once() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("nested/history.sqlite");
    let storage = LocalDb::open(&path).await.expect("open");
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    append(
        &storage,
        &mut meta,
        vec![chunk(Some("old")), independent(), independent()],
    )
    .await;
    storage
        .test_sql(localdb::TestSql::ClearHistoryIndex)
        .await
        .expect("simulate pre-index database");
    storage
        .window(meta.id, None, 1)
        .await
        .expect("initial index build");
    storage
        .test_sql(localdb::TestSql::CorruptFirstPayload)
        .await
        .expect("corrupt unrequested payload");
    drop(storage);
    let storage = LocalDb::open(&path).await.expect("reopen");
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("no rescan of old payload");
    assert_eq!(window.events.len(), 1);
    assert_eq!(window.cursor, meta.revision);
}
