use std::{sync::Arc, time::Duration};

use crate::{
    agent::{Catalog, LocalHost},
    model::{ContentBlock, Cursor, Event, EventKind, Message, Owner, Role, Timestamp},
};

use super::{AttachOptions, CreateRequest, Limits, Registry, Ring};

/// Longest a test waits for the placeholder agent to finish a turn.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn event(cursor: u64, kind: EventKind) -> Event {
    Event {
        session: crate::model::SessionId::new(),
        cursor: Cursor::from_raw(cursor),
        at: Timestamp::now(),
        kind,
    }
}

fn message(cursor: u64, text: &str) -> Event {
    event(cursor, EventKind::Message(Message::text(Role::Agent, text)))
}

fn registry() -> Registry {
    Registry::new(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
    )
}

async fn wait_for_turn(session: &super::Session) {
    let mut attachment = session.attach(AttachOptions::last(0));

    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match attachment.recv().await {
                Ok(event) if matches!(event.kind, EventKind::TurnFinished { .. }) => return,
                Ok(_) => continue,
                Err(error) => panic!("attachment failed: {error}"),
            }
        }
    })
    .await
    .expect("the placeholder agent finished the turn");
}

#[test]
fn ring_pages_strictly_after_a_cursor() {
    let mut ring = Ring::new(8);
    for cursor in 1..=5 {
        ring.push(message(cursor, &format!("m{cursor}")));
    }

    let page = ring.page(Cursor::START.next(), 2);

    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].cursor, Cursor::from_raw(2));
    assert_eq!(page.next, Some(Cursor::from_raw(3)));
    assert!(page.has_more());
}

#[test]
fn ring_reports_the_end_of_history() {
    let mut ring = Ring::new(8);
    ring.push(message(1, "only"));

    let page = ring.page(Cursor::START, 10);

    assert_eq!(page.events.len(), 1);
    assert_eq!(page.next, None);
    assert!(!page.has_more());
}

#[test]
fn ring_evicts_the_oldest_events() {
    let mut ring = Ring::new(2);
    for cursor in 1..=3 {
        ring.push(message(cursor, &format!("m{cursor}")));
    }

    assert_eq!(ring.len(), 2);
    assert_eq!(ring.tail(10)[0].cursor, Cursor::from_raw(2));
}

#[test]
fn ring_counts_only_messages_for_replay() {
    let mut ring = Ring::new(8);
    ring.push(message(0, "user"));
    ring.push(event(
        1,
        EventKind::TurnStarted {
            turn: Default::default(),
        },
    ));
    ring.push(message(2, "agent"));

    let messages = ring.messages(1);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].cursor, Cursor::from_raw(2));
}

#[tokio::test]
async fn attaching_replays_recent_messages_and_then_follows_live() {
    let registry = registry();
    let session = registry
        .create(CreateRequest {
            agent: "embedded".into(),
            owner: Owner::current(),
            cwd: None,
            title: Some("replay test".to_owned()),
        })
        .await
        .unwrap();

    session
        .prompt(vec![ContentBlock::text("hello")])
        .await
        .unwrap();
    wait_for_turn(&session).await;

    let attachment = session.attach(AttachOptions::last(10));
    let texts: Vec<String> = attachment
        .backlog()
        .iter()
        .filter_map(|event| event.message())
        .map(Message::plain_text)
        .collect();

    assert!(texts.iter().any(|text| text == "hello"));
    assert!(texts.iter().any(|text| text.contains("echo agent")));
    assert_eq!(session.meta().status, crate::model::SessionStatus::Idle);
}

#[tokio::test]
async fn detaching_does_not_stop_the_agent() {
    let registry = registry();
    let session = registry
        .create(CreateRequest {
            agent: "embedded".into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .unwrap();

    session
        .prompt(vec![ContentBlock::text("first")])
        .await
        .unwrap();
    wait_for_turn(&session).await;
    drop(session.attach(AttachOptions::last(0)));

    session
        .prompt(vec![ContentBlock::text("second")])
        .await
        .unwrap();
    wait_for_turn(&session).await;

    let attachment = session.attach(AttachOptions::last(10));
    let texts: Vec<String> = attachment
        .backlog()
        .iter()
        .filter_map(|event| event.message())
        .map(Message::plain_text)
        .collect();

    assert!(texts.iter().any(|text| text == "first"));
    assert!(texts.iter().any(|text| text == "second"));
}

#[tokio::test]
async fn removing_a_session_stops_it_and_hides_it() {
    let registry = registry();
    let session = registry
        .create(CreateRequest {
            agent: "embedded".into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .unwrap();
    let id = session.id();

    registry.remove(id).await.unwrap();

    assert!(registry.get(id).await.is_err());
    assert!(registry.is_empty().await);
    assert_eq!(session.meta().status, crate::model::SessionStatus::Stopped);
}

#[tokio::test]
async fn unknown_backends_are_rejected_at_creation() {
    let registry = registry();
    let error = registry
        .create(CreateRequest {
            agent: "nope".into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .err()
        .expect("the backend is not configured");

    assert!(error.to_string().contains("not configured"));
}
