use super::*;
use async_trait::async_trait;
use basr::model::{Owner, SessionId, SessionMeta};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAgent(Arc<AtomicUsize>);

#[async_trait]
impl agent::Agent for CountingAgent {
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }

    async fn shutdown(&mut self) {}
}

async fn session() -> (Arc<Session>, Arc<AtomicUsize>) {
    let session = Session::new(
        SessionMeta::new(SessionId::new(), "counting".into(), Owner::current()),
        Limits {
            history_capacity: std::num::NonZeroUsize::new(4).expect("capacity"),
            ..Limits::default()
        },
        None,
    );
    let count = Arc::new(AtomicUsize::new(0));
    session
        .ready(Box::new(CountingAgent(count.clone())))
        .await
        .expect("ready");
    (session, count)
}

fn submission(session: &Session) -> PromptSubmission {
    PromptSubmission::new(acp::PromptRequest::new(
        session.id().to_string(),
        vec!["hello".into()],
    ))
}

#[test]
fn canonical_fingerprint_preserves_numbers_attachments_and_metadata() {
    fn request(meta: &str) -> acp::PromptRequest {
        serde_json::from_str(&format!(
            r#"{{"sessionId":"session","prompt":[{{"type":"image","data":"AAAA","mimeType":"image/png"}}],"_meta":{meta}}}"#
        )).expect("ACP request")
    }
    let first = request(r#"{"z":{"b":2,"a":1},"n":9007199254740992}"#);
    let reordered = request(r#"{"n":9007199254740992,"z":{"a":1,"b":2}}"#);
    assert_eq!(fingerprint(&first), fingerprint(&reordered));
    assert_ne!(
        fingerprint(&first),
        fingerprint(&request(r#"{"z":{"b":2,"a":1},"n":9007199254740993}"#))
    );
    assert_ne!(
        fingerprint(&first),
        fingerprint(&request(r#"{"z":{"b":3,"a":1},"n":9007199254740992}"#))
    );
    let mut changed = serde_json::to_value(&first).expect("request");
    changed["prompt"][0]["data"] = "BBBB".into();
    assert_ne!(
        fingerprint(&first),
        fingerprint(&serde_json::from_value(changed).expect("attachment"))
    );
}

#[tokio::test]
async fn concurrent_identity_retries_dispatch_once_and_conflicts_have_no_effect() {
    let (session, count) = session().await;
    let request = submission(&session);
    let (first, second) = tokio::join!(
        session.accept_prompt(request.clone()),
        session.accept_prompt(request.clone())
    );
    let receipt = first.expect("first");
    assert_eq!(second.expect("retry"), receipt);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let before = session.snapshot();
    let history = session.page(None, 100).expect("history");
    assert_eq!(
        history
            .events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::TurnStarted { .. }))
            .count(),
        1
    );
    let mut conflict = request.clone();
    conflict.request.prompt = vec!["different".into()];
    assert!(matches!(
        session.accept_prompt(conflict).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: Kind::Conflict,
            ..
        }))
    ));
    assert_eq!(session.snapshot(), before);
    assert_eq!(session.page(None, 100).expect("history"), history);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    session.shutdown().await;
}

#[tokio::test]
async fn rejected_requests_leave_no_acceptance_or_history() {
    let (session, _) = session().await;
    let mut empty = submission(&session);
    empty.request.prompt.clear();
    let before = session.snapshot();
    assert!(matches!(
        session.accept_prompt(empty.clone()).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: Kind::InvalidRequest,
            ..
        }))
    ));
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        session.prompt_acceptance(empty.turn).await.expect("lookup"),
        None
    );
    session
        .accept_prompt(submission(&session))
        .await
        .expect("active");
    for kind in [Kind::Busy, Kind::Archived] {
        if kind == Kind::Archived {
            session.try_set_archived(true).await.expect("archive");
        }
        let rejected = submission(&session);
        let before = session.snapshot();
        let history = session.page(None, 100).expect("history");
        assert!(matches!(session.accept_prompt(rejected.clone()).await,
            Err(SubmissionError::Rejected(PromptRejection { kind: actual, .. })) if actual == kind));
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.page(None, 100).expect("history"), history);
        assert_eq!(
            session
                .prompt_acceptance(rejected.turn)
                .await
                .expect("lookup"),
            None
        );
    }
    session.shutdown().await;
    let inactive = submission(&session);
    let before = session.snapshot();
    assert!(matches!(
        session.accept_prompt(inactive.clone()).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: Kind::Unavailable,
            ..
        }))
    ));
    assert_eq!(
        session
            .prompt_acceptance(inactive.turn)
            .await
            .expect("lookup"),
        None
    );
    assert_eq!(session.snapshot(), before);
}

#[tokio::test]
async fn acceptance_survives_eviction_archival_and_runtime_failure() {
    let (session, count) = session().await;
    let request = submission(&session);
    let receipt = session
        .accept_prompt(request.clone())
        .await
        .expect("accepted");
    for title in ["one", "two", "three", "four", "five"] {
        session.set_title(title).await.expect("title");
    }
    assert!(
        !session
            .page(None, 100)
            .expect("history")
            .events
            .iter()
            .any(|event| matches!(event.kind, EventKind::TurnStarted { .. }))
    );
    session.try_set_archived(true).await.expect("archive");
    assert_eq!(
        session
            .accept_prompt(request.clone())
            .await
            .expect("archived retry"),
        receipt
    );
    session.fail("backend died".into()).await;
    assert_eq!(
        session
            .prompt_acceptance(request.turn)
            .await
            .expect("lookup"),
        Some(receipt)
    );
    assert_eq!(
        session.accept_prompt(request).await.expect("failed retry"),
        receipt
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    session.shutdown().await;
}

#[tokio::test]
async fn failed_runtime_leaves_new_submission_unknown_but_preserves_existing_acceptance() {
    let (session, count) = session().await;
    let accepted = submission(&session);
    let receipt = session
        .accept_prompt(accepted.clone())
        .await
        .expect("accepted");
    session.persistence_failed(storage::Error::WorkerClosed);
    let before = session.snapshot();
    let history = session.page(None, 100).expect("history");
    let request = submission(&session);
    assert!(matches!(
        session.accept_prompt(request.clone()).await,
        Err(SubmissionError::Unknown(SessionError::Runtime(
            RuntimeFailure::Persistence { .. }
        )))
    ));
    assert_eq!(
        session
            .prompt_acceptance(request.turn)
            .await
            .expect("lookup"),
        None
    );
    assert_eq!(
        session
            .prompt_acceptance(accepted.turn)
            .await
            .expect("accepted lookup"),
        Some(receipt)
    );
    assert_eq!(
        session
            .accept_prompt(accepted)
            .await
            .expect("existing receipt wins"),
        receipt
    );
    assert_eq!(session.snapshot(), before);
    assert_eq!(session.page(None, 100).expect("history"), history);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    session.shutdown().await;
}
