use super::*;

#[test]
fn withdrawing_a_message_returns_it_to_the_editor_and_holds_the_composition() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    assert_eq!(
        outbox
            .withdraw(0, draft("composition"))
            .expect("withdraw A")
            .retrieved
            .expect("return A to the editor")
            .text,
        "A"
    );
    assert!(outbox.editing_active());
    assert_eq!(outbox.editing(), None);
    assert!(
        outbox.advance(&idle(1), true).is_none(),
        "the detached edit pauses the queue"
    );
    assert_eq!(
        outbox.cancel_edit().expect("restore composition").text,
        "composition"
    );
    assert_eq!(outbox.front().expect("B remains queued").text, "B");
}

#[test]
fn sending_a_withdrawn_message_returns_it_to_its_queue_position() {
    let mut outbox = SessionOutbox::default();
    for text in ["A", "B", "C"] {
        outbox.enqueue(draft(text));
    }
    assert_eq!(
        outbox
            .withdraw(1, draft("composition"))
            .expect("withdraw B")
            .retrieved
            .expect("return B to the editor")
            .text,
        "B"
    );
    assert_eq!(
        outbox
            .enqueue(draft("B'"))
            .expect("restore composition")
            .text,
        "composition"
    );
    assert_eq!(
        outbox
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B'", "C"],
        "submitting a withdrawn edit keeps its queue position"
    );
}

#[test]
fn immediate_send_waits_for_cancel_and_turn_completion_without_resuming_paused_drafts() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.pause();
    let mut meta = idle(1);
    meta.status = SessionStatus::Working;
    let previous_turn = TurnId::new();
    meta.active_turn = Some(previous_turn);
    assert!(outbox.prioritize(1, true));
    assert!(
        outbox.advance(&idle(2), true).is_none(),
        "cancel must complete first"
    );
    outbox.complete_interrupt(true);
    assert!(
        outbox.advance(&meta, true).is_none(),
        "working turn must finish first"
    );
    let send = outbox.advance(&idle(2), true).expect("send B");
    assert_eq!(send.draft.text, "B");
    let submission = submission(send);
    let receipt = TurnReceipt {
        turn: submission.turn,
        cursor: Cursor::from_raw(3),
    };
    outbox.settle(submission, Ok(PromptSubmissionOutcome::Accepted(receipt)));
    outbox.observe(&finish(&meta, receipt.turn, 4));
    assert!(
        outbox.advance(&idle(4), true).is_none(),
        "other drafts remain paused"
    );
    assert_eq!(outbox.front().expect("A remains queued").text, "A");
}

#[test]
fn completion_before_cancel_acknowledgement_never_releases_the_pending_send_early() {
    for succeeded in [true, false] {
        let mut outbox = SessionOutbox::default();
        let meta = idle(1);
        let submission = submission(outbox.start(draft("current turn"), Origin::Composer));
        let accepted = TurnReceipt {
            turn: submission.turn,
            cursor: Cursor::from_raw(1),
        };
        outbox.settle(submission, Ok(PromptSubmissionOutcome::Accepted(accepted)));
        outbox.enqueue(draft("A"));
        outbox.enqueue(draft("B"));
        outbox.pause();
        outbox.prioritize(1, true);
        outbox.observe(&finish(&meta, accepted.turn, 2));
        assert!(
            outbox.advance(&idle(2), true).is_none(),
            "completion still waits for cancel acknowledgement"
        );
        outbox.complete_interrupt(succeeded);
        if succeeded {
            assert_eq!(sent(outbox.advance(&idle(2), true)), "B");
        } else {
            assert!(outbox.advance(&idle(2), true).is_none());
            assert!(outbox.paused());
            assert_eq!(outbox.front().expect("priority draft retained").text, "B");
        }
    }
}
