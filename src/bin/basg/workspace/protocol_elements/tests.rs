use super::*;

#[test]
fn cancelled_plan_entries_are_not_remaining_work() {
    let plans = [acp2::PlanUpdate::new(acp2::PlanUpdateContent::Items(
        acp2::PlanItems::new(
            "release",
            vec![
                acp2::PlanEntry::new(
                    "done",
                    acp2::PlanEntryPriority::High,
                    acp2::PlanEntryStatus::Completed,
                ),
                acp2::PlanEntry::new(
                    "cancelled",
                    acp2::PlanEntryPriority::Low,
                    acp2::PlanEntryStatus::Cancelled,
                ),
                acp2::PlanEntry::new(
                    "pending",
                    acp2::PlanEntryPriority::Medium,
                    acp2::PlanEntryStatus::Pending,
                ),
            ],
        ),
    ))];
    assert_eq!(plan_remaining(&plans), 1);
}

#[test]
fn compaction_summary_display_preserves_annotation_metadata_and_nontext_boundaries() {
    let annotations = acp2::Annotations::new().priority(0.5);
    let meta = serde_json::json!({"source":"agent"})
        .as_object()
        .expect("metadata object")
        .clone();
    let chunks = vec![
        acp2::ContentBlock::Text(acp2::TextContent::new("first")),
        acp2::ContentBlock::Text(
            acp2::TextContent::new("annotated").annotations(annotations.clone()),
        ),
        acp2::ContentBlock::Text(
            acp2::TextContent::new("metadata")
                .annotations(annotations.clone())
                .meta(meta.clone()),
        ),
        acp2::ContentBlock::ResourceLink(acp2::ResourceLink::new(
            "artifact",
            "file:///tmp/artifact",
        )),
        acp2::ContentBlock::Text(
            acp2::TextContent::new("after resource")
                .annotations(annotations)
                .meta(meta),
        ),
    ];
    assert_eq!(compaction_summary_blocks(&chunks), chunks);
}
