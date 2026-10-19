use super::*;
mod archive;
mod correlation;
mod interruption;
mod pending;
use bas::{
    acp,
    model::{Owner, SessionId},
};

fn draft(text: &str) -> Draft {
    Draft {
        text: text.into(),
        attachments: Vec::new(),
    }
}

fn idle(revision: u64) -> SessionMeta {
    let mut meta = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    meta.status = SessionStatus::Idle;
    meta.revision = Cursor::from_raw(revision);
    meta
}

fn finish(meta: &SessionMeta, turn: TurnId, cursor: u64) -> Event {
    Event::new(
        meta.id,
        Cursor::from_raw(cursor),
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        },
    )
}

fn sent(send: Option<Send>) -> String {
    send.expect("expected send").draft.text
}

fn submission(send: Send) -> Arc<PromptSubmission> {
    Arc::new(PromptSubmission {
        turn: send.turn,
        request: acp::PromptRequest::new("test-session", vec![send.draft.text.into()]),
    })
}

fn rejected() -> Result<PromptSubmissionOutcome, String> {
    Ok(PromptSubmissionOutcome::Rejected(
        bas::model::PromptRejection {
            kind: bas::model::PromptRejectionKind::Busy,
            message: "busy".into(),
        },
    ))
}

#[test]
fn events_before_receipt_wait_for_acceptance_and_match_the_turn() {
    let mut outbox = SessionOutbox::default();
    let meta = idle(0);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    let submission = submission(outbox.advance(&meta, true).expect("send A"));
    let turn = submission.turn;
    outbox.observe(&finish(&meta, turn, 3));
    assert!(outbox.advance(&meta, true).is_none());
    outbox.settle(
        submission,
        Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
            turn,
            cursor: Cursor::from_raw(2),
        })),
    );
    assert_eq!(sent(outbox.advance(&meta, true)), "B");
}

#[test]
fn unknown_retains_only_its_latest_completion_and_ignores_other_turns() {
    let meta = idle(1);
    let mut outbox = SessionOutbox::default();
    let submission = submission(outbox.start(draft("original"), Origin::Composer));
    let turn = submission.turn;
    outbox.settle(submission.clone(), Err("transport failed".into()));
    for cursor in 2..100 {
        outbox.observe(&finish(&meta, TurnId::new(), cursor));
    }
    assert!(matches!(
        &outbox.dispatch,
        Dispatch::Submitting { finished: None, .. }
    ));
    outbox.observe(&finish(&meta, turn, 101));
    outbox.observe(&finish(&meta, turn, 100));
    outbox.observe(&finish(&meta, TurnId::new(), 102));
    assert!(
        matches!(&outbox.dispatch, Dispatch::Submitting { finished: Some(completion), .. }
        if completion.cursor.raw() == 101)
    );
    outbox.settle(
        submission,
        Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
            turn,
            cursor: Cursor::from_raw(2),
        })),
    );
    assert!(matches!(outbox.dispatch, Dispatch::Ready));
}

#[test]
fn receipts_need_the_specific_turn_even_after_newer_idle_catalogs() {
    let mut outbox = SessionOutbox::default();
    let meta = idle(100);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    let submission = submission(outbox.advance(&meta, true).expect("send A"));
    let turn = submission.turn;
    outbox.settle(
        submission.clone(),
        Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
            turn,
            cursor: Cursor::from_raw(1),
        })),
    );
    outbox.observe(&finish(&meta, TurnId::new(), 101));
    assert!(outbox.advance(&meta, true).is_none());
    // A duplicated receipt must not reset Awaiting.
    outbox.settle(
        submission,
        Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
            turn,
            cursor: Cursor::from_raw(1),
        })),
    );
    assert!(outbox.advance(&meta, true).is_none());
    outbox.observe(&finish(&meta, turn, 102));
    assert_eq!(sent(outbox.advance(&meta, true)), "B");
}

#[test]
fn cancelled_edits_and_failed_sends_keep_the_original_draft() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.begin_edit(draft("composition"));
    assert_eq!(outbox.cancel_edit().expect("restore").text, "composition");
    assert_eq!(outbox.front().expect("original").text, "A");
    assert!(outbox.advance(&idle(0), true).is_none());
    outbox.begin_edit(draft(""));
    outbox.enqueue(draft("A'"));
    let submission = submission(outbox.advance(&idle(0), true).expect("send edited A"));
    outbox.settle(submission, rejected());
    assert_eq!(outbox.front().expect("failed draft").text, "A'");
    assert_eq!(outbox.len(), 2);
    assert!(outbox.advance(&idle(0), true).is_none());
}
