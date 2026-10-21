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

fn prompt(id: &str) -> EventKind {
    update(json!({"sessionUpdate":"user_message_chunk", "messageId":id,
        "content":{"type":"text","text":"prompt"}}))
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
            prompt("first"),
            chunk(Some("cross-turn")),
            EventKind::TurnStarted {
                turn: TurnId::new(),
            },
            independent(),
            chunk(Some("cross-turn")),
            EventKind::TurnStarted {
                turn: TurnId::new(),
            },
            independent(),
        ],
    )
    .await;
    let latest = storage
        .window(meta.id, None, 1)
        .await
        .expect("independent suffix");
    assert_eq!(latest.events, events[6..]);
    assert_eq!(latest.before, Some(events[6].cursor));
    let older = storage
        .window(meta.id, latest.before, 1)
        .await
        .expect("dependency expansion");
    assert_eq!(older.events, events[1..6]);
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
        events[6..]
    );
    assert!(matches!(
        storage.window(meta.id, Some(events[7].cursor), 1).await,
        Err(Error::NotTurnBoundary(_))
    ));
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
        prompt("terminal"),
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
        prompt("compaction"),
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
            prompt("after-removal"),
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
            prompt("replacement"),
            plan("a", "replacement"),
            prompt("latest"),
            independent(),
        ],
    )
    .await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("ordered plans");
    assert_eq!(
        window.context,
        vec![events[0].clone(), events[1].clone(), events[3].clone()]
    );
    let window = storage
        .window(meta.id, Some(events[4].cursor), 1)
        .await
        .expect("replacement in page");
    assert_eq!(window.context, events[..2]);
    assert_eq!(window.events, events[2..4]);
    let next = append(
        storage,
        &mut meta,
        vec![
            update(json!({"sessionUpdate":"plan_removed","planId":"a"})),
            plan("a", "reinserted"),
            prompt("reinserted"),
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
        vec![
            prompt("a"),
            tool("a"),
            prompt("b"),
            tool("b"),
            prompt("c"),
            tool("a"),
            prompt("d"),
            tool("b"),
        ],
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
    let previous = append(storage, &mut meta, vec![prompt("old"), chunk(Some("old"))]).await;
    let turn = TurnId::new();
    let mut kinds = vec![
        EventKind::TurnStarted { turn },
        prompt("long"),
        prompt("long"),
    ];
    kinds.extend((0..300).flat_map(|id| {
        [
            update(json!({
                "sessionUpdate":"tool_call","toolCallId":format!("tool-{id}"),"title":"independent"
            })),
            chunk(Some("long-answer")),
            update(
                json!({"sessionUpdate":"agent_thought_chunk","messageId":"long-thought",
                "content":{"type":"text","text":"thinking"}}),
            ),
        ]
    }));
    kinds.push(EventKind::TurnFinished {
        turn,
        result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
    });
    let events = append(storage, &mut meta, kinds).await;
    let window = storage
        .window(meta.id, None, 1)
        .await
        .expect("whole long turn");
    assert_eq!(window.events, events);
    assert!(window.context.is_empty());
    assert_eq!(window.meta, meta);
    assert_eq!(window.cursor, meta.revision);
    let older = storage
        .window(meta.id, window.before, 1)
        .await
        .expect("previous turn");
    assert_eq!(older.events, previous);
    assert_eq!(older.meta, meta);
    assert_eq!(older.cursor, meta.revision);
    assert_eq!(older.before, None);
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
async fn long_turn_is_returned_whole_without_previous_turn() {
    long_turn_contract(&Memory::new(NonZeroUsize::new(1024).expect("capacity"))).await;
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
            prompt("new"),
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
    append(
        &storage,
        &mut meta,
        vec![EventKind::TurnStarted {
            turn: TurnId::new(),
        }],
    )
    .await;
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
    let events = append(
        &storage,
        &mut meta,
        vec![
            EventKind::TurnStarted {
                turn: TurnId::new(),
            },
            usage(),
        ],
    )
    .await;
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
    let events = append(
        &storage,
        &mut meta,
        (0..200)
            .flat_map(|turn| {
                let mut kinds = vec![prompt(&format!("prompt-{turn}"))];
                kinds.extend((0..30).map(|_| chunk(Some(&format!("answer-{turn}")))));
                kinds
            })
            .collect(),
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
    assert_eq!(window.events, events[events.len() - 31..]);
    assert_eq!(window.before, Some(events[events.len() - 31].cursor));
    assert_eq!(window.meta, meta);
    assert_eq!(window.cursor, meta.revision);
    let older = storage
        .window(meta.id, window.before, 1)
        .await
        .expect("older indexed turn");
    assert_eq!(older.events, events[events.len() - 62..events.len() - 31]);
    assert_eq!(older.meta, meta);
    assert_eq!(older.cursor, meta.revision);
}

async fn imported_turns_contract(storage: &dyn Storage) {
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    let legacy = || {
        update(json!({"sessionUpdate":"user_message_chunk",
        "content":{"type":"text","text":"legacy prompt chunk"}}))
    };
    let events = append(
        storage,
        &mut meta,
        vec![
            independent(),
            prompt("first"),
            prompt("first"),
            chunk(Some("answer")),
            legacy(),
            legacy(),
            chunk(None),
            legacy(),
            legacy(),
            chunk(None),
        ],
    )
    .await;
    let recent = storage
        .window(meta.id, None, 1)
        .await
        .expect("last imported turn");
    assert_eq!(recent.events, events[7..]);
    assert_eq!(recent.before, Some(events[7].cursor));
    let older = storage
        .window(meta.id, recent.before, 1)
        .await
        .expect("older imported turn");
    assert_eq!(older.events, events[4..7]);
    assert_eq!(older.before, Some(events[4].cursor));
    let first = storage
        .window(meta.id, older.before, 1)
        .await
        .expect("identified multi-chunk prompt");
    assert_eq!(first.events, events[1..4]);
    assert_eq!(first.before, Some(events[1].cursor));
    for window in [&recent, &older, &first] {
        assert_eq!(window.meta, meta);
        assert_eq!(window.cursor, meta.revision);
        assert!(window.context.is_empty());
    }
    let initial = storage
        .window(meta.id, first.before, 1)
        .await
        .expect("initial segment");
    assert_eq!(initial.events, events[..1]);
    assert_eq!(initial.before, None);
    let two = storage.window(meta.id, None, 2).await.expect("two turns");
    assert_eq!(two.events, events[4..]);
    assert!(matches!(
        storage.window(meta.id, Some(events[8].cursor), 1).await,
        Err(Error::NotTurnBoundary(_))
    ));
}

#[tokio::test]
async fn imported_multichunk_prompts_page_as_complete_turns() {
    imported_turns_contract(&Memory::new(NonZeroUsize::new(64).expect("capacity"))).await;
    imported_turns_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

#[tokio::test]
async fn memory_never_returns_a_turn_with_an_evicted_start() {
    let storage = Memory::new(NonZeroUsize::new(3).expect("capacity"));
    let mut meta = state();
    storage.create(meta.clone(), None).await.expect("create");
    append(
        &storage,
        &mut meta,
        vec![
            EventKind::TurnStarted {
                turn: TurnId::new(),
            },
            independent(),
            independent(),
            independent(),
        ],
    )
    .await;
    assert!(matches!(storage.window(meta.id, None, 1).await,
        Err(Error::Expired { requested, .. }) if requested.raw() == 1));
}
