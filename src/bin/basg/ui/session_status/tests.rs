use bas::model::{Cursor, Owner, SessionId};

use super::*;

#[test]
fn status_uses_reply_cursor_not_catalog_revision() {
    let mut session = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    session.status = SessionStatus::Idle;
    assert_eq!(Status::for_session(&session, 0, false), Status::Idle);
    session.last_reply = Cursor::from_raw(9);
    assert_eq!(Status::for_session(&session, 8, false), Status::Unread);
    assert_eq!(Status::for_session(&session, 9, false), Status::Idle);
    for (state, expected) in [
        (SessionStatus::Working, Status::Working),
        (SessionStatus::Starting, Status::Starting),
        (SessionStatus::Failed, Status::Failed),
        (SessionStatus::Stopped, Status::Stopped),
    ] {
        session.status = state;
        assert_eq!(Status::for_session(&session, 0, true), expected);
        assert_eq!(Status::for_snapshot(&session.clone().into()), expected);
    }
    session.status = SessionStatus::Idle;
    session.last_reply = Cursor::from_raw(0);
    assert_eq!(Status::for_session(&session, 0, true), Status::Unread);
    assert_eq!(Status::for_snapshot(&session.into()), Status::Idle);
}

#[test]
fn runtime_failure_uses_the_snapshots_observable_status() {
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    meta.status = SessionStatus::Idle;
    let mut snapshot = SessionSnapshot::from(meta);
    snapshot.runtime.failure = Some(bas::model::RuntimeFailure::Persistence {
        message: "storage unavailable".into(),
    });
    assert_eq!(snapshot.meta.status, SessionStatus::Idle);
    assert_eq!(Status::for_snapshot(&snapshot), Status::Failed);
}
