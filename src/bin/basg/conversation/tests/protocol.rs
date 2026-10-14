use super::*;

fn update(conversation: &mut Conversation, cursor: u64, update: acp::SessionUpdate) {
    apply(conversation, cursor, EventKind::Acp(update));
}

fn compaction<'a>(conversation: &'a Conversation, id: &str) -> &'a Compaction {
    conversation
        .items
        .iter()
        .find_map(|item| match item {
            Item::Compaction(compaction) if compaction.id.to_string() == id => Some(compaction),
            _ => None,
        })
        .expect("retained compaction")
}

#[test]
fn compaction_replay_preserves_position_and_patch_semantics() {
    let mut live = conversation();
    let updates = vec![
        acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
            "cmp-1",
            "first".into(),
        )),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("after".into())),
        acp::SessionUpdate::CompactionUpdate(
            acp::CompactionUpdate::new("cmp-1", acp::CompactionStatus::Failed)
                .error("provider failed")
                .meta(
                    serde_json::json!({"source":"provider"})
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
        ),
        acp::SessionUpdate::CompactionUpdate(acp::CompactionUpdate::new(
            "cmp-1",
            acp::CompactionStatus::InProgress,
        )),
        acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
            "cmp-1",
            " second".into(),
        )),
        acp::SessionUpdate::CompactionUpdate(
            acp::CompactionUpdate::new("cmp-1", acp::CompactionStatus::Completed)
                .summary(vec!["replacement".into()])
                .error(None::<String>)
                .meta(None::<acp::Meta>),
        ),
    ];
    let events: Vec<_> = updates
        .into_iter()
        .enumerate()
        .map(|(index, update)| {
            Event::new(
                live.session.id,
                Cursor::from_raw(index as u64 + 1),
                EventKind::Acp(update),
            )
        })
        .collect();
    for event in &events[..5] {
        live.apply(event.clone());
    }
    let retained = compaction(&live, "cmp-1");
    assert_eq!(retained.summary.len(), 2);
    assert_eq!(retained.error.as_deref(), Some("provider failed"));
    assert!(retained.meta.is_some());
    live.apply(events[5].clone());

    // Reconnecting starts with current metadata, then replays earlier content events.
    let mut replay = Conversation::new(live.session.clone());
    for event in events {
        replay.apply(event);
    }
    assert_eq!(compaction(&live, "cmp-1"), compaction(&replay, "cmp-1"));
    let retained = compaction(&replay, "cmp-1");
    assert_eq!(retained.status, acp2::CompactionStatus::Completed);
    assert_eq!(
        retained.summary,
        vec![acp2::ContentBlock::from("replacement")]
    );
    assert!(retained.error.is_none());
    assert!(retained.meta.is_none());
    assert_eq!(replay.items.len(), 2);
    assert!(matches!(replay.items[0], Item::Compaction(_)));
    assert!(
        matches!(&replay.items[1], Item::Message { message, .. } if message.plain_text() == "after")
    );
    assert_eq!(replay.rows.len(), 2);
}

#[test]
fn compaction_null_summary_clears_and_duplicate_cursor_does_not_append() {
    let mut conversation = conversation();
    let chunk = acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
        "cmp",
        "summary".into(),
    ));
    update(&mut conversation, 1, chunk.clone());
    update(&mut conversation, 1, chunk);
    assert_eq!(compaction(&conversation, "cmp").summary.len(), 1);
    update(
        &mut conversation,
        2,
        acp::SessionUpdate::CompactionUpdate(
            acp::CompactionUpdate::new("cmp", acp::CompactionStatus::Completed)
                .summary(None::<Vec<acp::ContentBlock>>),
        ),
    );
    assert!(compaction(&conversation, "cmp").summary.is_empty());
    assert_eq!(conversation.items.len(), 1);
}

#[test]
fn multiple_plans_replace_by_id_and_removal_survives_replay() {
    let mut conversation = conversation();
    let updates = [
        acp::SessionUpdate::Plan(acp::Plan::new(vec![acp::PlanEntry::new(
            "legacy",
            acp::PlanEntryPriority::High,
            acp::PlanEntryStatus::Pending,
        )])),
        acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(acp::PlanUpdateContent::markdown(
            "notes", "## Notes",
        ))),
        acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(acp::PlanUpdateContent::file(
            "release",
            "file:///tmp/plan.md",
        ))),
        acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(acp::PlanUpdateContent::markdown(
            "notes",
            "## Updated",
        ))),
        acp::SessionUpdate::Plan(acp::Plan::new(vec![acp::PlanEntry::new(
            "legacy updated",
            acp::PlanEntryPriority::High,
            acp::PlanEntryStatus::Completed,
        )])),
        acp::SessionUpdate::PlanRemoved(acp::PlanRemoved::new("release")),
        acp::SessionUpdate::UsageUpdate(
            acp::UsageUpdate::new(50, 200).cost(acp::Cost::new(0.5, "USD")),
        ),
        acp::SessionUpdate::UsageUpdate(
            acp::UsageUpdate::new(100, 200).cost(acp::Cost::new(0.75, "USD")),
        ),
    ];
    for (index, plan) in updates.iter().enumerate() {
        update(&mut conversation, index as u64 + 1, plan.clone());
    }
    let mut replay = Conversation::new(conversation.session.clone());
    for (index, plan) in updates.into_iter().enumerate() {
        update(&mut replay, index as u64 + 1, plan);
    }
    assert_eq!(replay.plans, conversation.plans);
    assert_eq!(replay.usage, conversation.usage);
    assert_eq!(replay.usage.expect("latest usage").used, 100);
    assert_eq!(conversation.plans.len(), 2);
    assert!(
        matches!(&conversation.plans[0].plan, acp2::PlanUpdateContent::Items(plan)
            if plan.entries.len() == 1
                && plan.entries[0].content == "legacy updated"
                && plan.entries[0].status == acp2::PlanEntryStatus::Completed)
    );
    assert!(
        matches!(&conversation.plans[1].plan, acp2::PlanUpdateContent::Markdown(plan) if plan.content == "## Updated")
    );
    assert!(
        conversation.items.is_empty(),
        "plan updates are independent of transcript rows"
    );
}

#[test]
fn protocol_notices_are_transient_and_separate_from_local_turn_notices() {
    let mut conversation = conversation();
    update(
        &mut conversation,
        1,
        acp::SessionUpdate::Notice(acp::Notice::new(acp::NoticeSeverity::Warning, "old notice")),
    );
    assert!(
        conversation.notices.is_empty(),
        "history never restores transient notices"
    );
    assert!(conversation.items.is_empty());
    update(
        &mut conversation,
        0,
        acp::SessionUpdate::Notice(
            acp::Notice::new(acp::NoticeSeverity::Warning, "provider warning")
                .description("retry later"),
        ),
    );
    assert_eq!(
        conversation.cursor, 0,
        "transient delivery does not consume history cursors"
    );
    assert_eq!(conversation.session.revision, Cursor::START);
    let first = conversation.notices[0].id;
    conversation.notices.clear();
    conversation.push_notice(acp2::Notice::new(acp2::NoticeSeverity::Info, "ready"));
    assert!(
        conversation.notices[0].id > first,
        "dismissed banner IDs cannot be reused"
    );
    assert!(conversation.items.is_empty());
    let turn = TurnId::new();
    apply(&mut conversation, 2, EventKind::TurnStarted { turn });
    apply(
        &mut conversation,
        3,
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::Cancelled)),
        },
    );
    assert!(matches!(conversation.items[0], Item::Notice { .. }));
    assert_eq!(conversation.notices.len(), 1);
}
