use bas::model::{PromptRejection, PromptRejectionKind as Kind, PromptSubmission};
use sha2::{Digest, Sha256};

use super::*;

#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error(transparent)]
    Rejected(#[from] PromptRejection),
    #[error("submission acceptance is unknown: {0}")]
    Unknown(#[from] SessionError),
}

fn rejected(kind: Kind, message: impl Into<String>) -> SubmissionError {
    PromptRejection {
        kind,
        message: message.into(),
    }
    .into()
}

fn fingerprint(request: &acp::PromptRequest) -> [u8; 32] {
    let mut value = serde_json::to_value(request).expect("ACP request serializes");
    value.sort_all_objects();
    Sha256::digest(serde_json::to_vec(&value).expect("JSON value serializes")).into()
}

impl Session {
    /// Read-only lookup, independent of runtime health and backend restoration.
    ///
    /// # Errors
    /// Returns storage read failures, never an inferred rejection.
    pub async fn prompt_acceptance(
        &self,
        turn: TurnId,
    ) -> Result<Option<TurnReceipt>, SessionError> {
        let _gate = self.transaction.lock().await;
        Ok(self
            .acceptance_locked(turn)
            .await?
            .map(|accepted| accepted.receipt))
    }

    /// # Errors
    /// Rejects conflicting reuse of an identity; storage failures leave acceptance unknown.
    pub async fn known_submission(
        &self,
        submission: &PromptSubmission,
    ) -> Result<Option<TurnReceipt>, SubmissionError> {
        self.validate_target(submission)?;
        let fingerprint = fingerprint(&submission.request);
        let _gate = self.transaction.lock().await;
        self.known_submission_locked(submission.turn, fingerprint)
            .await
    }

    fn validate_target(&self, submission: &PromptSubmission) -> Result<(), SubmissionError> {
        if submission
            .request
            .session_id
            .to_string()
            .parse::<SessionId>()
            .ok()
            != Some(self.id())
        {
            return Err(rejected(
                Kind::InvalidRequest,
                "prompt belongs to another session",
            ));
        }
        Ok(())
    }

    async fn acceptance_locked(
        &self,
        turn: TurnId,
    ) -> Result<Option<storage::PromptAcceptance>, SessionError> {
        let (storage, id) = {
            let state = self.lock();
            if state.deleted {
                return Err(storage::Error::NotFound(state.meta.id).into());
            }
            let Some(storage) = &state.storage else {
                return Ok(state.acceptances.get(&turn).copied());
            };
            (Arc::clone(storage), state.meta.id)
        };
        Ok(storage.acceptance(id, turn).await?)
    }

    async fn known_submission_locked(
        &self,
        turn: TurnId,
        fingerprint: [u8; 32],
    ) -> Result<Option<TurnReceipt>, SubmissionError> {
        let Some(accepted) = self.acceptance_locked(turn).await? else {
            return Ok(None);
        };
        if accepted.fingerprint != fingerprint {
            return Err(rejected(
                Kind::Conflict,
                "submission id was already used with different prompt content",
            ));
        }
        Ok(Some(accepted.receipt))
    }

    /// Register a submission exactly once before handing it to the backend.
    ///
    /// # Errors
    /// Explicit preflight refusal is Rejected. A failed lookup or commit leaves
    /// acceptance Unknown. After registration, dispatch failure cannot revoke the receipt.
    pub async fn accept_prompt(
        self: &Arc<Self>,
        submission: PromptSubmission,
    ) -> Result<TurnReceipt, SubmissionError> {
        self.validate_target(&submission)?;
        let fingerprint = fingerprint(&submission.request);
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            if let Some(receipt) = session
                .known_submission_locked(submission.turn, fingerprint)
                .await?
            {
                return Ok(receipt);
            }
            let receipt = {
                let state = session.lock();
                state
                    .require_ready()
                    .map_err(|error| match error {
                        SessionError::Inactive(_) => {
                            rejected(Kind::Unavailable, error.to_string())
                        }
                        error => SubmissionError::Unknown(error),
                    })?;
                if state.meta.archived {
                    return Err(rejected(Kind::Archived, "session is archived"));
                }
                if state.active.is_some() {
                    return Err(rejected(Kind::Busy, "a turn is already running"));
                }
                if submission.request.prompt.is_empty() {
                    return Err(rejected(
                        Kind::InvalidRequest,
                        "a prompt must contain at least one content block",
                    ));
                }
                TurnReceipt {
                    turn: submission.turn,
                    cursor: state.cursor.next(),
                }
            };
            let request = submission.request;
            let mut events = vec![EventKind::TurnStarted { turn: receipt.turn }];
            events.extend(request.prompt.iter().cloned().map(|content| {
                EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(content).meta(request.meta.clone()),
                ))
            }));
            let acceptance = storage::PromptAcceptance {
                receipt,
                fingerprint,
            };
            let mut meta = session.meta();
            session
                .publish_submission_locked(events, &mut meta, Some(acceptance))
                .await?;
            let dispatch = {
                let mut state = session.lock();
                state.active = Some(receipt.turn);
                state
                    .agent
                    .as_mut()
                    .expect("ready backend")
                    .prompt(receipt.turn, request)
            };
            if let Err(error) = dispatch {
                let interrupted = if matches!(error, agent::Error::Backend { .. }) {
                    session.fail_locked(error.to_string()).await
                } else {
                    session
                        .interrupt_locked(
                            InterruptionReason::DispatchFailure {
                                message: error.to_string(),
                            },
                            None,
                        )
                        .await
                };
                if let Err(error) = interrupted {
                    tracing::error!(turn = %receipt.turn, %error, "accepted submission could not record its interruption");
                }
            }
            Ok(receipt)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
