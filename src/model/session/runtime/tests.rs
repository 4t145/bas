use super::*;
use crate::model::{Cursor, Owner, SessionId};

#[test]
fn runtime_failure_does_not_rewrite_committed_metadata() {
    let mut meta = SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current());
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(TurnId::new());
    meta.revision = Cursor::from_raw(7);
    let serialized = serde_json::to_value(&meta).expect("stored metadata");
    let mut snapshot = SessionSnapshot::from(meta.clone());
    let runtime = snapshot.runtime.id;
    assert_eq!(snapshot.status(), SessionStatus::Working);
    assert_eq!(snapshot.active_turn(), meta.active_turn);

    snapshot.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "fixture commit failure".into(),
    });
    assert_eq!(snapshot.status(), SessionStatus::Failed);
    assert_eq!(snapshot.active_turn(), None);
    assert_eq!(snapshot.meta, meta);
    assert_eq!(snapshot.runtime.id, runtime);
    assert_eq!(
        serde_json::to_value(&snapshot.meta).expect("metadata"),
        serialized
    );

    let recovered = SessionSnapshot::from(meta.clone());
    assert_ne!(recovered.runtime.id, runtime);
    assert!(recovered.runtime.failure.is_none());
    assert_eq!(recovered.meta, meta);
}

#[test]
fn healthy_runtime_preserves_each_committed_lifecycle_state() {
    for status in [
        SessionStatus::Starting,
        SessionStatus::Idle,
        SessionStatus::Working,
        SessionStatus::Stopped,
        SessionStatus::Failed,
    ] {
        let mut meta = SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current());
        meta.set_status(status);
        let snapshot = SessionSnapshot::from(meta.clone());
        assert_eq!(snapshot.status(), status);
        assert_eq!(snapshot.active_turn(), meta.active_turn);
        assert!(snapshot.runtime.failure.is_none());
    }
}
