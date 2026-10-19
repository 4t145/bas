use super::*;

fn interruption(meta: &SessionMeta, turn: TurnId, cursor: u64) -> Event {
    Event::new(
        meta.id,
        Cursor::from_raw(cursor),
        EventKind::TurnInterrupted {
            turn,
            reason: bas::model::InterruptionReason::DaemonRestarted,
        },
    )
}

#[test]
fn interruption_and_receipt_in_either_order_release_but_pause() {
    for receipt_first in [false, true] {
        for archived in [false, true] {
            let mut meta = idle(1);
            let mut outbox = SessionOutbox::default();
            outbox.enqueue(draft("accepted"));
            outbox.enqueue(draft("unsent"));
            let submission = submission(outbox.advance(&meta, true).expect("send"));
            let receipt = TurnReceipt {
                turn: submission.turn,
                cursor: Cursor::from_raw(2),
            };
            if receipt_first {
                outbox.settle(
                    submission.clone(),
                    Ok(PromptSubmissionOutcome::Accepted(receipt)),
                );
            }
            meta.archived = archived;
            outbox.advance(&meta, true);
            let event = interruption(&meta, receipt.turn, 3);
            outbox.observe(&event);
            meta.apply_event(&event);
            if !receipt_first {
                assert!(outbox.sending());
                outbox.settle(
                    submission.clone(),
                    Ok(PromptSubmissionOutcome::Accepted(receipt)),
                );
            }
            assert!(matches!(outbox.dispatch, Dispatch::Ready));
            assert!(outbox.paused());
            assert_eq!(
                outbox.front().expect("unsent draft retained").text,
                "unsent"
            );
            meta.archived = false;
            assert!(outbox.advance(&meta, true).is_none());
            outbox.resume();
            assert_eq!(sent(outbox.advance(&meta, true)), "unsent");
        }
    }
}

#[test]
fn unrelated_or_old_interruption_never_releases_or_pauses_current_send() {
    for receipt_first in [false, true] {
        let meta = idle(1);
        let mut outbox = SessionOutbox::default();
        outbox.enqueue(draft("accepted"));
        outbox.enqueue(draft("unsent"));
        let submission = submission(outbox.advance(&meta, true).expect("send"));
        let receipt = TurnReceipt {
            turn: submission.turn,
            cursor: Cursor::from_raw(3),
        };
        if receipt_first {
            outbox.settle(
                submission.clone(),
                Ok(PromptSubmissionOutcome::Accepted(receipt)),
            );
        }
        outbox.observe(&interruption(&meta, TurnId::new(), 4));
        outbox.observe(&interruption(&meta, receipt.turn, 2));
        if !receipt_first {
            outbox.settle(
                submission.clone(),
                Ok(PromptSubmissionOutcome::Accepted(receipt)),
            );
        }
        assert!(!outbox.paused());
        assert!(outbox.advance(&meta, true).is_none());
        outbox.observe(&finish(&meta, receipt.turn, 5));
        assert_eq!(sent(outbox.advance(&meta, true)), "unsent");
    }
}
