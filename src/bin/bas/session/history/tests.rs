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
