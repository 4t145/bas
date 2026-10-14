use super::*;
use bas::model::{Owner, RuntimeFailure, SessionSnapshot};

#[test]
fn session_queries_follow_prompt_runtime_and_removal_state() {
    let id = SessionId::new();
    let other = SessionId::new();
    let mut store = SessionStore::default();
    store.unstarted.extend([id, other]);
    assert!(store.healthy(id));
    assert!(store.is_draft(id));
    store.record_prompt(id);
    assert!(!store.is_draft(id));
    assert!(store.is_draft(other));

    let mut snapshot = SessionSnapshot::from(SessionMeta::new(id, "test".into(), Owner::current()));
    snapshot.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "disk full".into(),
    });
    store.runtimes.observe(&snapshot, true);
    assert!(!store.healthy(id));
    store.mark_removed(id);
    assert!(store.runtimes.failure(id).is_none());
    assert!(
        !store.healthy(id),
        "removing the runtime must not make a deleted session healthy"
    );
    assert!(store.healthy(other));
    assert!(store.is_draft(other));
}

#[test]
fn removal_and_content_discard_preserve_independent_session_state() {
    let removed = SessionId::new();
    let retained = SessionId::new();
    let mut store = SessionStore::default();
    for id in [removed, retained] {
        let mut snapshot =
            SessionSnapshot::from(SessionMeta::new(id, "test".into(), Owner::current()));
        snapshot.runtime.failure = Some(RuntimeFailure::Persistence {
            message: "disk full".into(),
        });
        store.entries.push(snapshot.meta.clone());
        store.runtimes.observe(&snapshot, true);
        store.observed.insert(id);
        store.drafts.insert(id, Draft::default());
        store.outboxes.insert(id, outbox::SessionOutbox::default());
        store.unstarted.insert(id);
        store.prompted.insert(id);
        store.seen_replies.insert(id, 7);
        store.pending.configuring.insert(id);
        store.pending.cancelling.insert(id);
        store.pending.archiving.insert(id);
        store.pending.deleting.insert(id);
        store.pending.loading_attachments.insert(id);
    }

    store.mark_removed(removed);

    assert_eq!(store.removed, HashSet::from([removed]));
    assert!(store.runtimes.failure(removed).is_none());
    assert!(store.runtimes.failure(retained).is_some());
    assert!(store.drafts.contains_key(&removed));
    assert!(store.outboxes.contains_key(&removed));
    assert!(store.unstarted.contains(&removed));
    assert!(store.prompted.contains(&removed));

    store.discard_content(removed);

    assert!(!store.drafts.contains_key(&removed));
    assert!(!store.outboxes.contains_key(&removed));
    assert!(!store.unstarted.contains(&removed));
    assert!(!store.prompted.contains(&removed));
    assert!(store.drafts.contains_key(&retained));
    assert!(store.outboxes.contains_key(&retained));
    assert!(store.unstarted.contains(&retained));
    assert!(store.prompted.contains(&retained));
    assert_eq!(store.entries.len(), 2);
    assert_eq!(store.observed, HashSet::from([removed, retained]));
    assert_eq!(
        store.seen_replies,
        HashMap::from([(removed, 7), (retained, 7)])
    );
    for pending in [
        &store.pending.configuring,
        &store.pending.cancelling,
        &store.pending.archiving,
        &store.pending.deleting,
        &store.pending.loading_attachments,
    ] {
        assert_eq!(pending, &HashSet::from([removed, retained]));
    }
}

#[test]
fn observing_catalog_ids_reports_disappearance_without_discarding_content() {
    let disappeared = SessionId::new();
    let retained = SessionId::new();
    let added = SessionId::new();
    let mut store = SessionStore {
        observed: HashSet::from([disappeared, retained]),
        ..Default::default()
    };
    store.drafts.insert(disappeared, Draft::default());
    store
        .outboxes
        .insert(disappeared, outbox::SessionOutbox::default());
    store.seen_replies.insert(disappeared, 7);

    assert_eq!(
        store.observe_ids(HashSet::from([retained, added])),
        vec![disappeared]
    );
    assert_eq!(store.observed, HashSet::from([retained, added]));
    assert!(store.removed.is_empty());
    assert!(store.drafts.contains_key(&disappeared));
    assert!(store.outboxes.contains_key(&disappeared));
    assert_eq!(store.seen_replies[&disappeared], 7);
    assert!(
        store
            .observe_ids(HashSet::from([retained, added]))
            .is_empty()
    );
}

#[test]
fn recording_prompt_only_changes_that_sessions_prompt_status() {
    let id = SessionId::new();
    let other = SessionId::new();
    let mut store = SessionStore {
        unstarted: HashSet::from([id, other]),
        ..Default::default()
    };
    store.drafts.insert(id, Draft::default());
    store.pending.configuring.insert(id);

    store.record_prompt(id);
    store.record_prompt(id);

    assert_eq!(store.prompted, HashSet::from([id]));
    assert_eq!(store.unstarted, HashSet::from([other]));
    assert!(store.drafts.contains_key(&id));
    assert!(store.pending.configuring.contains(&id));
}

#[test]
fn removing_directory_draft_removes_content_and_metadata_together() {
    let directory = Some(PathBuf::from("/workspace"));
    let mut drafts = DirectoryDrafts::default();
    for key in [None, directory.clone()] {
        drafts.contents.insert(key.clone(), Draft::default());
        drafts
            .metadata
            .insert(key, navigation::draft_row::Metadata::default());
    }

    drafts.remove(&directory);

    assert!(!drafts.contents.contains_key(&directory));
    assert!(!drafts.metadata.contains_key(&directory));
    assert!(drafts.contents.contains_key(&None));
    assert!(drafts.metadata.contains_key(&None));

    drafts.remove(&None);
    drafts.remove(&directory);
    assert!(drafts.contents.is_empty());
    assert!(drafts.metadata.is_empty());
}
