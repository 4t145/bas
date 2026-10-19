use super::*;

fn accepted(turn: TurnId, cursor: u64) -> Result<PromptSubmissionOutcome, String> {
    Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
        turn,
        cursor: Cursor::from_raw(cursor),
    }))
}

#[test]
fn invalid_receipts_retain_the_submission_until_a_valid_acceptance() {
    for wrong_turn in [false, true] {
        let mut outbox = SessionOutbox::default();
        outbox.enqueue(draft("original"));
        outbox.enqueue(draft("next"));
        let original = submission(outbox.advance(&idle(1), true).expect("send"));
        assert!(outbox.composition_locked());
        let result = if wrong_turn {
            accepted(TurnId::new(), 2)
        } else {
            accepted(original.turn, 0)
        };
        assert!(outbox.settle(original.clone(), result).is_none());
        assert!(outbox.sending());
        assert!(!outbox.composition_locked());
        assert!(Arc::ptr_eq(outbox.unknown().expect("unknown"), &original));
        assert_eq!(outbox.front().expect("next remains queued").text, "next");
        assert!(outbox.advance(&idle(3), true).is_none());
        assert!(matches!(
            outbox.settle(original.clone(), accepted(original.turn, 2)),
            Some(Settlement::Accepted { origin: Origin::Queue, draft }) if draft.text == "original"
        ));
        assert!(outbox.unknown().is_none());
        assert!(outbox.advance(&idle(3), true).is_none());
        outbox.observe(&finish(&idle(3), original.turn, 3));
        assert_eq!(sent(outbox.advance(&idle(3), true)), "next");
    }
}

#[test]
fn stale_and_duplicate_results_never_mutate_a_new_submission() {
    let mut outbox = SessionOutbox::default();
    let previous = submission(outbox.start(draft("previous"), Origin::Composer));
    assert!(matches!(
        outbox.settle(previous.clone(), accepted(previous.turn, 1)),
        Some(Settlement::Accepted { .. })
    ));
    assert!(outbox.settle(previous.clone(), rejected()).is_none());
    assert!(
        outbox
            .settle(previous.clone(), accepted(previous.turn, 2))
            .is_none()
    );
    outbox.observe(&finish(&idle(2), previous.turn, 2));
    let current = submission(outbox.start(draft("current"), Origin::Composer));
    for result in [rejected(), accepted(previous.turn, 3), Err("lost".into())] {
        assert!(outbox.settle(previous.clone(), result).is_none());
        assert_eq!(outbox.turn(), Some(current.turn));
        assert!(outbox.composition_locked());
        assert!(!outbox.paused());
        assert!(outbox.unknown().is_none());
    }
    assert!(matches!(
        outbox.settle(current.clone(), accepted(current.turn, 3)),
        Some(Settlement::Accepted { origin: Origin::Composer, draft }) if draft.text == "current"
    ));
}

#[test]
fn mismatched_payload_never_replaces_or_settles_a_retained_submission() {
    for resolving in [false, true] {
        let mut outbox = SessionOutbox::default();
        let original = submission(outbox.start(draft("original"), Origin::Composer));
        outbox.settle(original.clone(), Err("lost".into()));
        if resolving {
            outbox.resolve();
        }
        let changed = Arc::new(PromptSubmission {
            turn: original.turn,
            request: acp::PromptRequest::new("test-session", vec!["changed".into()]),
        });
        for result in [Err("lost".into()), rejected(), accepted(original.turn, 2)] {
            assert!(outbox.settle(changed.clone(), result).is_none());
            assert!(Arc::ptr_eq(outbox.unknown().expect("original"), &original));
            assert_eq!(outbox.resolving(), resolving);
            assert!(!outbox.paused());
        }
        assert!(matches!(
            outbox.settle(original.clone(), accepted(original.turn, 2)),
            Some(Settlement::Accepted { draft, .. }) if draft.text == "original"
        ));
    }
}
