use super::*;

#[test]
fn withdrawing_an_earlier_message_keeps_the_edit_reserved_for_its_original_slot() {
    let mut outbox = SessionOutbox::default();
    for text in ["A", "B", "C"] {
        outbox.enqueue(draft(text));
    }
    assert_eq!(
        outbox
            .begin_edit_at(2, draft("composition"))
            .expect("edit")
            .text,
        "C"
    );
    assert!(
        outbox
            .withdraw(0)
            .expect("withdraw A")
            .composition
            .is_none()
    );
    assert_eq!(outbox.editing(), Some(1));
    assert_eq!(
        outbox
            .enqueue(draft("C'"))
            .expect("restore composition")
            .text,
        "composition"
    );
    assert_eq!(
        outbox
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["B", "C'"]
    );
}

#[test]
fn withdrawing_the_edited_message_restores_the_displaced_composition() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.begin_edit_at(1, draft("composition"));
    assert_eq!(
        outbox
            .withdraw(1)
            .expect("withdraw edited B")
            .composition
            .expect("restore")
            .text,
        "composition"
    );
    assert!(outbox.editing().is_none());
    assert_eq!(outbox.front().expect("A remains queued").text, "A");
    assert!(outbox.advance(&idle(1), true).is_none());
    outbox.resume();
    assert_eq!(sent(outbox.advance(&idle(1), true)), "A");
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
fn failed_interrupt_keeps_the_selected_message_and_never_dispatches_automatically() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.prioritize(1, true);
    outbox.complete_interrupt(false);
    assert_eq!(
        outbox
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    assert!(outbox.paused());
    assert!(outbox.advance(&idle(3), true).is_none());
    outbox.resume();
    assert_eq!(sent(outbox.advance(&idle(3), true)), "B");
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

#[test]
fn immediately_sending_an_edit_preserves_composition_and_the_other_paused_messages() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.begin_edit_at(1, draft("composition"));
    assert_eq!(
        outbox
            .enqueue_immediate(draft("B'"), false)
            .expect("restore composition")
            .text,
        "composition"
    );
    let send = outbox.advance(&idle(1), true).expect("send edited B");
    assert_eq!(send.draft.text, "B'");
    outbox.settle(submission(send), rejected());
    assert_eq!(
        outbox
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["B'", "A"]
    );
    assert!(outbox.advance(&idle(2), true).is_none());
}
