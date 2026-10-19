use bas::model::{Owner, SessionId, SessionMeta};

use super::Notification;
use crate::session::Completion;

#[test]
fn native_rejection_and_runtime_interruption_have_distinct_notifications() {
    use crate::session::CompletionOutcome;
    use bas::{acp, model::InterruptionReason};
    let session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    let native: Notification = Completion {
        session: session.clone(),
        outcome: CompletionOutcome::Native(Err(acp::Error::invalid_params())),
    }
    .into();
    let interrupted: Notification = Completion {
        session: session.clone(),
        outcome: CompletionOutcome::Interrupted(InterruptionReason::BackendFailure {
            message: "offline".into(),
        }),
    }
    .into();
    let cancelled: Notification = Completion {
        session,
        outcome: CompletionOutcome::Interrupted(InterruptionReason::CancelledBeforeDispatch),
    }
    .into();
    assert_eq!(native.body, "The agent rejected the prompt.");
    assert_eq!(
        interrupted.body,
        "Turn execution was interrupted by a runtime failure."
    );
    assert_eq!(cancelled.body, "The turn was cancelled before dispatch.");
    assert_eq!(native.urgency, super::Urgency::Critical);
    assert_eq!(interrupted.urgency, super::Urgency::Critical);
    assert_eq!(cancelled.urgency, super::Urgency::Low);
}
