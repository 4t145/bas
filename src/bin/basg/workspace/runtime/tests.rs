use super::*;
use bas::model::{Owner, SessionMeta};

#[test]
fn failure_latches_until_serial_catalog_replaces_runtime() {
    let healthy = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "test".into(),
        Owner::current(),
    ));
    let id = healthy.meta.id;
    let mut failed = healthy.clone();
    failed.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "disk full".into(),
    });
    let mut runtimes = Runtimes::default();
    runtimes.observe(&healthy, true);
    runtimes.observe(&failed, false);
    runtimes.observe(&healthy, false);
    runtimes.observe(&healthy, true);
    assert_eq!(runtimes.failure(id), failed.runtime.failure.as_ref());
    let restarted = SessionSnapshot::from(healthy.meta.clone());
    runtimes.observe(&restarted, false);
    assert!(runtimes.failure(id).is_some());
    runtimes.observe(&restarted, true);
    assert!(runtimes.failure(id).is_none());
    runtimes.observe(&failed, false);
    runtimes.observe(&healthy, false);
    assert!(runtimes.failure(id).is_none());
    assert_eq!(runtimes.0[&id].id, restarted.runtime.id);
}
