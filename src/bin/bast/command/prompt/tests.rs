use super::*;

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
