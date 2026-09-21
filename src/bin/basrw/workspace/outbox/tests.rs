use super::*;
mod archive;
use basr::{
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

fn sent(action: Option<Action>) -> String {
    let Some(Action::Send(draft)) = action else {
        panic!("expected send")
    };
    draft.text
}

#[test]
fn title_between_send_and_receipt_never_releases_the_next_draft() {
    let mut outbox = SessionOutbox::default();
    let mut meta = idle(1);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    assert_eq!(sent(outbox.advance(&meta, true)), "A");
    let title = Event::new(
        meta.id,
        Cursor::from_raw(2),
        EventKind::TitleChanged {
            title: "title".into(),
        },
    );
    meta.apply_event(&title);
    outbox.observe(&title);
    assert!(outbox.advance(&meta, true).is_none());
    let receipt = TurnReceipt {
        turn: TurnId::new(),
        cursor: Cursor::from_raw(3),
    };
    outbox.receipt(Some(receipt));
    assert!(outbox.advance(&meta, true).is_none());
    // Polling an unsubscribed session may prove completion, without a timestamp.
    meta.revision = Cursor::from_raw(4);
    assert_eq!(sent(outbox.advance(&meta, true)), "B");
}

#[test]
fn events_before_receipt_wait_for_acceptance_and_match_the_turn() {
    let mut outbox = SessionOutbox::default();
    let meta = idle(0);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.advance(&meta, true);
    let turn = TurnId::new();
    outbox.observe(&finish(&meta, turn, 3));
    assert!(outbox.advance(&meta, true).is_none());
    outbox.receipt(Some(TurnReceipt {
        turn,
        cursor: Cursor::from_raw(2),
    }));
    assert_eq!(sent(outbox.advance(&meta, true)), "B");
}

#[test]
fn legacy_receipts_need_the_specific_turn_even_after_newer_idle_catalogs() {
    let mut outbox = SessionOutbox::default();
    let meta = idle(100);
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    outbox.advance(&meta, true);
    let turn = TurnId::new();
    outbox.receipt(Some(TurnReceipt {
        turn,
        cursor: Cursor::START,
    }));
    outbox.observe(&finish(&meta, TurnId::new(), 101));
    assert!(outbox.advance(&meta, true).is_none());
    // A duplicated receipt must not reset Awaiting.
    outbox.receipt(Some(TurnReceipt {
        turn,
        cursor: Cursor::START,
    }));
    assert!(outbox.advance(&meta, true).is_none());
    outbox.observe(&finish(&meta, turn, 102));
    assert_eq!(sent(outbox.advance(&meta, true)), "B");
}

#[test]
fn edit_replaces_the_reserved_head_and_restores_the_displaced_composition() {
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft("A"));
    outbox.enqueue(draft("B"));
    assert_eq!(
        outbox.begin_edit(draft("composition")).expect("edit").text,
        "A"
    );
    assert!(outbox.advance(&idle(0), true).is_none());
    assert_eq!(
        outbox.enqueue(draft("A'")).expect("restore").text,
        "composition"
    );
    assert_eq!(sent(outbox.advance(&idle(0), true)), "A'");
    assert_eq!(outbox.front().expect("B").text, "B");
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
    outbox.advance(&idle(0), true);
    outbox.receipt(None);
    assert_eq!(outbox.front().expect("failed draft").text, "A'");
    assert_eq!(outbox.len(), 2);
    assert!(outbox.advance(&idle(0), true).is_none());
}

#[test]
fn attachment_payloads_survive_failure_edit_and_cancel() {
    let file = tempfile::NamedTempFile::new().expect("attachment file");
    std::fs::write(file.path(), "payload").expect("write attachment");
    let attachment = crate::attachment::Attachment::load(file.path()).expect("attachment");
    let draft = Draft {
        text: attachment.mention.clone(),
        attachments: vec![attachment.clone()],
    };
    let mut outbox = SessionOutbox::default();
    outbox.enqueue(draft.clone());
    outbox.advance(&idle(0), true);
    outbox.receipt(None);
    assert_eq!(
        outbox.front().expect("failed draft").attachments[0].id,
        attachment.id
    );
    let edit = outbox.begin_edit(draft).expect("edit");
    assert_eq!(edit.attachments[0].id, attachment.id);
    let restored = outbox.cancel_edit().expect("composition");
    assert_eq!(restored.attachments[0].id, attachment.id);
    assert_eq!(
        outbox.front().expect("original").attachments[0].id,
        attachment.id
    );
}
