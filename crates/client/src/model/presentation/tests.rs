use super::*;

#[test]
fn all_content_types_keep_payload_annotations_and_metadata_through_presentation() {
    let updates: Vec<acp::SessionUpdate> =
        serde_json::from_str(include_str!("../../service/assets/updates.json"))
            .expect("ACP content fixtures");
    let mut count = 0;
    for update in updates {
        if let acp::SessionUpdate::AgentMessageChunk(chunk) = update {
            let display = content_from_v1(&chunk.content).expect("v2 content");
            assert_eq!(content_to_v1(&display).expect("v1 content"), chunk.content);
            count += 1;
        }
    }
    assert_eq!(count, 5);
}
