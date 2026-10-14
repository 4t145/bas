use super::*;

#[test]
fn explicit_retry_preserves_identity_and_original_payload() {
    let first = submission("session", None, vec!["hello".into(), "world".into()]);
    let retry = submission(
        "session",
        Some(first.turn),
        vec!["hello".into(), "world".into()],
    );
    assert_eq!(first, retry);
    assert_ne!(
        first.turn,
        submission("session", None, vec!["hello".into(), "world".into()]).turn
    );
}

#[test]
fn unknown_output_explains_query_and_explicit_same_id_retry() {
    let turn = TurnId::new();
    let server = "[2001:db8::1]:4711".parse().expect("remote address");
    let message = unknown_message(server, "session", turn);
    assert!(message.contains("acceptance unknown"));
    assert!(message.contains(&format!(
        "bast --server {server} prompt-status session {turn}"
    )));
    assert!(message.contains(&format!(
        "bast --server {server} prompt session --submission {turn}"
    )));
    assert!(message.contains("exact original text"));
    assert!(message.contains("do not send the same request with a new id"));
    assert!(!message.contains("rejected"));
}

#[test]
fn accepted_prompt_and_status_preserve_stdout_turn_format() {
    let receipt = TurnReceipt {
        turn: TurnId::new(),
        cursor: bas::model::Cursor::from_raw(1),
    };
    let expected = format!("turn {}\n", receipt.turn).into_bytes();
    let mut output = Vec::new();
    report_outcome(PromptSubmissionOutcome::Accepted(receipt), &mut output)
        .expect("accepted output");
    assert_eq!(output, expected);
    output.clear();
    report_status(Some(receipt), receipt.turn, &mut output).expect("status output");
    assert_eq!(output, expected);
}

#[test]
fn missing_acceptance_remains_unknown_and_never_prints_an_accepted_turn() {
    let turn = TurnId::new();
    let mut output = Vec::new();
    report_status(None, turn, &mut output).expect("unknown output");
    let output = String::from_utf8(output).expect("UTF-8 output");
    assert!(output.contains(&format!("submission {turn}")));
    assert!(output.contains("acceptance unknown"));
    assert!(output.contains("not a rejection"));
    assert!(output.contains("does not rule out later acceptance"));
    assert!(output.contains("no prompt was sent"));
    assert!(!output.starts_with("turn "));
}

#[test]
fn explicit_rejection_is_not_unknown_or_success() {
    let mut output = Vec::new();
    let error = report_outcome(
        PromptSubmissionOutcome::Rejected(bas::model::PromptRejection {
            kind: bas::model::PromptRejectionKind::Busy,
            message: "already running".into(),
        }),
        &mut output,
    )
    .expect_err("explicit rejection");
    assert!(output.is_empty());
    assert_eq!(error.to_string(), "prompt rejected (Busy): already running");
}
