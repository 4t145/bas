use bas::model::{EventKind, SessionId, SessionStatus};

use super::*;

fn ring() -> Ring {
    let mut ring = Ring::new(NonZeroUsize::new(3).expect("capacity"));
    let session = SessionId::new();
    for cursor in 1..=4 {
        ring.push(Event::new(
            session,
            Cursor::from_raw(cursor),
            EventKind::StatusChanged {
                status: SessionStatus::Idle,
            },
        ));
    }
    ring
}

#[test]
fn visitor_checks_limits_and_eviction_before_calling_the_visitor() {
    let ring = ring();
    assert_eq!(
        ring.visit(None, 0, |_, _| panic!("invalid limit")),
        Err(HistoryError::EmptyPage)
    );
    assert!(matches!(
        ring.visit(Some(Cursor::START), 1, |_, _| panic!("expired cursor")),
        Err(HistoryError::Expired { .. })
    ));
}

#[test]
fn visitor_reports_more_beyond_limit_and_stops_without_cloning() {
    let ring = ring();
    let mut visited = Vec::new();
    ring.visit(None, 1, |event, more| {
        visited.push((event.cursor.raw(), more));
        true
    })
    .expect("limited visit");
    assert_eq!(visited, [(2, true)]);
    visited.clear();
    ring.visit(None, usize::MAX, |event, more| {
        visited.push((event.cursor.raw(), more));
        false
    })
    .expect("early stop");
    assert_eq!(visited, [(2, true)]);
    visited.clear();
    ring.visit(Some(Cursor::from_raw(2)), usize::MAX, |event, more| {
        visited.push((event.cursor.raw(), more));
        true
    })
    .expect("tail visit");
    assert_eq!(visited, [(3, true), (4, false)]);
    ring.visit(Some(Cursor::from_raw(4)), 1, |_, _| panic!("empty tail"))
        .expect("empty visit");
}
