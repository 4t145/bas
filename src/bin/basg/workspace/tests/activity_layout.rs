use super::*;
use bas::acp;
use base64::Engine;

struct Activity {
    group: String,
    thoughts: Vec<String>,
}

fn image() -> acp::ImageContent {
    let bytes = include_bytes!("../images/assets/sample.png");
    acp::ImageContent::new(
        base64::engine::general_purpose::STANDARD.encode(bytes),
        "image/png",
    )
    .uri("/missing/generated.png")
}

fn populate(fixture: &Fixture, include_image: bool, window: &mut Window, cx: &mut App) -> Activity {
    fixture.view.update(cx, |view, cx| {
        let session = view.selected.id.expect("session");
        let mut tool = acp::ToolCall::new("generated", "Image generation").status(acp::ToolCallStatus::Completed);
        if include_image {
            tool = tool.content(vec![
                acp::ToolCallContent::Content(acp::Content::new("Tool detail")),
                acp::ToolCallContent::Content(acp::Content::new(ContentBlock::Image(image()))),
            ]);
        }
        let updates = [
            acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new("Generate an image".into())),
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new("First thought".into()).message_id("first")),
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new("Second thought".into()).message_id("second")),
            acp::SessionUpdate::ToolCall(tool),
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new("Last thought".into()).message_id("last")),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("Here is the generated result.".into())),
        ];
        for (index, update) in updates.into_iter().enumerate() {
            view.update(Update::Event {
                epoch: view.selected.epoch,
                event: Event::new(session, Cursor::from_raw(index as u64 + 1), EventKind::Acp(update)),
            }, window, cx);
        }
        let conversation = view.selected.timeline.conversation.as_ref().expect("conversation");
        let thoughts: Vec<_> = conversation.items.iter().filter(|item|
            matches!(item, Item::Message { message, .. } if message.role == bas::model::Role::Thought)
        ).map(Item::id).collect();
        assert_eq!(thoughts.len(), 3);
        view.composer.expanded = false;
        cx.notify();
        Activity { group: format!("activity-{}", thoughts[0]), thoughts }
    })
}

#[gpui_kit::test]
fn thoughts_and_tools_share_header_columns_height_and_compact_gaps(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let activity = populate(&fixture, false, window, cx);
        for compact in [false, true] {
            for font_size in [14., 22., 28.] {
                cx.global_mut::<theme::Appearance>().content_font_size = font_size;
                fixture.view.update(cx, |view, cx| {
                    view.window.compact = compact;
                    view.window.output_open = true;
                    view.selected
                        .timeline
                        .expanded
                        .insert(activity.group.clone());
                    view.resize_window(window, cx);
                });
                window.bounds_changed(cx);
                window.render_frame(cx);
                let tool = window.find("expand-tool-generated").bounds();
                let tool_label = window.find("disclosure-label-tool-generated").bounds();
                let tool_kind = window.find("tool-kind-tool-generated").bounds();
                for thought in &activity.thoughts {
                    let header = window.find(format!("expand-{thought}")).bounds();
                    let label = window.find(format!("disclosure-label-{thought}")).bounds();
                    let kind = window.find(format!("thought-status-{thought}")).bounds();
                    assert_eq!(header.left(), tool.left(), "shared disclosure column");
                    assert_eq!(
                        header.size.height, tool.size.height,
                        "same header height at {font_size}px"
                    );
                    assert_eq!(label.left(), tool_label.left(), "shared title column");
                    assert_eq!(kind.left(), tool_kind.left(), "shared icon column");
                    assert_eq!(kind.center().y, header.center().y);
                }
                let first = window
                    .find(format!("expand-{}", activity.thoughts[0]))
                    .bounds();
                let second = window
                    .find(format!("expand-{}", activity.thoughts[1]))
                    .bounds();
                // Spacing tokens scale with rem size; every adjacent header must
                // still share the same small gap, without per-kind padding.
                let gap = second.top() - first.bottom();
                let last = window
                    .find(format!("expand-{}", activity.thoughts[2]))
                    .bounds();
                assert_eq!(tool.top() - second.bottom(), gap);
                assert_eq!(last.top() - tool.bottom(), gap);
                assert!(
                    gap > px(0.) && gap <= first.size.height * 0.25,
                    "compact header gaps"
                );
                assert!(
                    tool_label.size.height >= px(font_size),
                    "header preserves content font height"
                );
            }
        }
    })
    .expect("uniform activity headers");
}

#[gpui_kit::test]
fn tool_images_remain_visible_when_groups_tools_and_output_details_are_collapsed(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    let activity = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            let activity = populate(&fixture, true, window, cx);
            window.render_frame(cx);
            activity
        })
        .expect("receive native tool image");
    cx.run_until_parked();
    for (group_open, tool_open, output_open) in [
        (false, false, true),
        (true, false, true),
        (true, true, true),
        (true, true, false),
        (false, true, true),
    ] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                for (key, open) in [
                    (activity.group.clone(), group_open),
                    ("tool-generated".into(), tool_open),
                    ("output-closed-tool-generated".into(), !output_open),
                ] {
                    if open {
                        view.selected.timeline.expanded.insert(key);
                    } else {
                        view.selected.timeline.expanded.remove(&key);
                    }
                }
                cx.notify();
            });
            window.render_frame(cx);
        })
        .expect("set disclosure state");
        cx.run_until_parked();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let image = window.find("image-tool-generated-content-1");
            assert!(
                image.visible(),
                "image is not hidden by disclosure state {group_open}/{tool_open}/{output_open}"
            );
            assert!(image.bounds().size.height > px(0.));
            if !group_open {
                assert!(window.try_find("expand-tool-generated").is_none());
            }
        })
        .expect("native image stays available outside hidden details");
    }
}
