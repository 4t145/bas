use super::*;

#[test]
fn prepared_payload_is_independent_of_later_draft_edits() {
    let session = SessionId::new();
    let turn = TurnId::new();
    let mut text = String::from("original");
    let prepared = prepare(
        session,
        turn,
        &text,
        &[],
        &acp::PromptCapabilities::default(),
    );
    let retry = prepared.clone();
    text.replace_range(.., "edited");
    assert!(Arc::ptr_eq(&prepared, &retry));
    assert_eq!(retry.turn, turn);
    assert_eq!(
        retry.request.prompt,
        vec![acp::ContentBlock::from("original")]
    );
    assert_eq!(retry.request.session_id.to_string(), session.to_string());
}
