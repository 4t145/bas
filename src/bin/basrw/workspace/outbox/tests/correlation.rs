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
fn local_enqueue_failure_rejects_only_the_matching_initial_send() {
    for origin in [Origin::Composer, Origin::Queue] {
        let mut outbox = SessionOutbox::default();
        let send = if origin == Origin::Queue {
            outbox.enqueue(draft("original"));
            outbox.advance(&idle(1), true).expect("send")
        } else {
            outbox.start(draft("original"), origin)
        };
        assert!(outbox.not_sent(TurnId::new(), "stale".into()).is_none());
        assert!(outbox.composition_locked());
        assert!(matches!(
            outbox.not_sent(send.turn, "closed".into()),
            Some(Settlement::Rejected { origin: actual, draft, message })
                if actual == origin && draft.text == "original" && message == "closed"
        ));
        assert!(outbox.paused());
        assert!(!outbox.sending());
        assert_eq!(outbox.len(), usize::from(origin == Origin::Queue));
        assert!(outbox.not_sent(send.turn, "duplicate".into()).is_none());
        if origin == Origin::Queue {
            assert_eq!(outbox.front().expect("requeued once").text, "original");
        }
    }
}

#[test]
fn resolve_failures_keep_the_original_payload_and_never_reject_locally() {
    let mut outbox = SessionOutbox::default();
    let original = submission(outbox.start(draft("original"), Origin::Composer));
    outbox.settle(original.clone(), Err("lost".into()));
    assert!(!outbox.composition_locked());
    assert!(outbox.not_sent(original.turn, "too late".into()).is_none());
    let retry = outbox.resolve().expect("resolve");
    assert!(Arc::ptr_eq(&retry, &original));
    assert!(outbox.resolving());
    assert!(outbox.resolve().is_none());
    assert!(outbox.not_sent(original.turn, "too late".into()).is_none());
    outbox.resolution_not_sent(TurnId::new());
    assert!(outbox.resolving());
    outbox.resolution_not_sent(original.turn);
    assert!(!outbox.resolving());
    assert!(Arc::ptr_eq(outbox.unknown().expect("retained"), &original));
    let retry = outbox.resolve().expect("resolve again");
    // Equality, not allocation identity, correlates a returned immutable payload.
    outbox.settle(Arc::new((*retry).clone()), Err("lost again".into()));
    assert!(!outbox.resolving());
    assert!(Arc::ptr_eq(outbox.unknown().expect("retained"), &original));
    assert!(!outbox.paused());
    assert!(outbox.sending());
    assert!(matches!(
        outbox.settle(original, rejected()),
        Some(Settlement::Rejected { origin: Origin::Composer, draft, message })
            if draft.text == "original" && message == "busy"
    ));
    assert!(outbox.paused());
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

#[test]
fn acceptance_returns_the_owned_draft_not_the_returned_request() {
    let mut outbox = SessionOutbox::default();
    let mut send = outbox.start(draft("owned"), Origin::Composer);
    send.draft.text = "changed by caller".into();
    let submission = submission(send);
    assert!(matches!(
        outbox.settle(submission.clone(), accepted(submission.turn, 1)),
        Some(Settlement::Accepted { draft, .. }) if draft.text == "owned"
    ));
}
