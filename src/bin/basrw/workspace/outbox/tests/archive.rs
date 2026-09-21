use super::*;

#[test]
fn archive_keeps_accepted_turn_and_restore_requires_explicit_retry() {
    let mut outbox = SessionOutbox::default();
    let mut meta = idle(1);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.advance(&meta, true);
    let receipt = TurnReceipt {
        turn: TurnId::new(),
        cursor: Cursor::from_raw(2),
    };
    outbox.receipt(Some(receipt));
    meta.archived = true;
    meta.revision = Cursor::from_raw(3);
    assert!(outbox.advance(&meta, true).is_none());
    assert!(matches!(outbox.dispatch, Dispatch::Awaiting(_)));
    assert_eq!(outbox.front().expect("queued").text, "B");
    meta.archived = false;
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(receipt.turn);
    assert!(outbox.advance(&meta, true).is_none());
    assert!(matches!(outbox.dispatch, Dispatch::Awaiting(_)));
    outbox.observe(&finish(&meta, receipt.turn, 4));
    meta.status = SessionStatus::Idle;
    meta.active_turn = None;
    assert!(outbox.advance(&meta, true).is_none());
    assert!(outbox.paused());
    outbox.begin_edit(draft("composition"));
    outbox.enqueue(draft("B edited"));
    assert_eq!(sent(outbox.advance(&meta, true)), "B edited");
}

#[test]
fn archive_while_submitting_retains_receipt_correlation() {
    let mut outbox = SessionOutbox::default();
    let mut meta = idle(1);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.advance(&meta, true);
    meta.archived = true;
    outbox.advance(&meta, true);
    let receipt = TurnReceipt {
        turn: TurnId::new(),
        cursor: Cursor::from_raw(2),
    };
    assert!(matches!(
        outbox.receipt(Some(receipt)),
        Some(Action::Accepted(Origin::Queue))
    ));
    assert!(matches!(outbox.dispatch, Dispatch::Awaiting(_)));
    assert!(outbox.paused());
    outbox.observe(&finish(&meta, receipt.turn, 3));
    assert!(matches!(outbox.dispatch, Dispatch::Ready));
    assert_eq!(outbox.front().expect("queued").text, "B");
}
