use super::*;
use basr::acp;

#[gpui_kit::test]
fn plans_usage_compaction_and_dismissible_notices_are_visible(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let updates = [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new("start".into())),
                acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(
                    acp::PlanUpdateContent::items(
                        "release",
                        vec![
                            acp::PlanEntry::new(
                                "check tests",
                                acp::PlanEntryPriority::High,
                                acp::PlanEntryStatus::Completed,
                            ),
                            acp::PlanEntry::new(
                                "publish",
                                acp::PlanEntryPriority::Low,
                                acp::PlanEntryStatus::Pending,
                            ),
                        ],
                    ),
                )),
                acp::SessionUpdate::UsageUpdate(
                    acp::UsageUpdate::new(100, 1000).cost(acp::Cost::new(0.05, "USD")),
                ),
                acp::SessionUpdate::CompactionUpdate(
                    acp::CompactionUpdate::new("cmp", acp::CompactionStatus::Completed)
                        .summary(vec!["**Retained summary**".into()]),
                ),
            ];
            for (index, update) in updates.into_iter().enumerate() {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            meta.id,
                            Cursor::from_raw(index as u64 + 1),
                            EventKind::Acp(update),
                        ),
                    },
                    window,
                    cx,
                );
            }
            let conversation = view.conversation.as_ref().expect("conversation");
            let cursor = conversation.cursor;
            let revision = conversation.session.revision;
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::START,
                        EventKind::Acp(acp::SessionUpdate::Notice(
                            acp::Notice::new(acp::NoticeSeverity::Warning, "Provider delayed")
                                .description("Retry shortly"),
                        )),
                    ),
                },
                window,
                cx,
            );
            let conversation = view.conversation.as_ref().expect("conversation");
            assert_eq!(conversation.cursor, cursor);
            assert_eq!(conversation.session.revision, revision);
            view.composer_expanded = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("plan-release-status-0").label(),
            Some("已完成 · 高优先级")
        );
        assert_eq!(
            window.find("plan-release-status-1").label(),
            Some("待完成 · 低优先级")
        );
        assert!(window.find("session-usage").bounds().size.height > px(0.));
        assert_eq!(
            window.find("expand-compaction-cmp").label(),
            Some("上下文压缩完成")
        );
        window.click("expand-compaction-cmp", cx);
        window.render_frame(cx);
        assert!(
            fixture
                .view
                .read(cx)
                .markdown
                .borrow()
                .contains_key("compaction-cmp-summary-0")
        );
        assert!(window.find("protocol-notice-1").bounds().size.height > px(0.));
        window.click("dismiss-protocol-notice-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("protocol-notice-1").is_none());
        let conversation = fixture
            .view
            .read(cx)
            .conversation
            .as_ref()
            .expect("conversation");
        assert!(conversation.notices.is_empty());
        assert_eq!(
            conversation.items.len(),
            2,
            "banner dismissal does not affect history"
        );
    })
    .expect("protocol elements render");
}

#[gpui_kit::test]
fn compaction_summary_markdown_remains_continuous_across_chunks(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let updates = [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new("start".into())),
                acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
                    "streamed",
                    "**first ".into(),
                )),
                acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
                    "streamed",
                    "second**".into(),
                )),
                acp::SessionUpdate::CompactionUpdate(acp::CompactionUpdate::new(
                    "streamed",
                    acp::CompactionStatus::Completed,
                )),
            ];
            for (index, update) in updates.into_iter().enumerate() {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            meta.id,
                            Cursor::from_raw(index as u64 + 1),
                            EventKind::Acp(update),
                        ),
                    },
                    window,
                    cx,
                );
            }
            view.composer_expanded = false;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("expand-compaction-streamed", cx);
        window.render_frame(cx);
    })
    .expect("streamed compaction render");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        let documents = view.markdown.borrow();
        let document = documents["compaction-streamed-summary-0"].state.clone();
        assert!(
            !documents.contains_key("compaction-streamed-summary-1"),
            "matching text chunks share one Markdown document"
        );
        let compaction = view
            .conversation
            .as_ref()
            .expect("conversation")
            .items
            .iter()
            .find_map(|item| {
                if let Item::Compaction(compaction) = item {
                    Some(compaction)
                } else {
                    None
                }
            })
            .expect("compaction");
        assert_eq!(
            compaction.summary,
            vec![
                basr::acp_v2::ContentBlock::from("**first "),
                basr::acp_v2::ContentBlock::from("second**")
            ]
        );
        drop(documents);
        document.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(
                state.selected_text().trim(),
                "first second",
                "Markdown syntax spanning chunks renders continuously"
            );
        });
    })
    .expect("continuous Markdown summary");
}
