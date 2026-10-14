use crate::acp;

use super::{TurnId, TurnReceipt};

/// Client-owned identity and immutable ACP payload for one Bas submission.
/// Retries reuse this value; a new user action gets a new identity.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptSubmission {
    pub turn: TurnId,
    pub request: acp::PromptRequest,
}

impl PromptSubmission {
    pub fn new(request: acp::PromptRequest) -> Self {
        Self {
            turn: TurnId::new(),
            request,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptRejectionKind {
    InvalidRequest,
    Busy,
    Archived,
    Unavailable,
    Conflict,
}

/// An explicit refusal of this submission attempt, not an ACP execution result.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PromptRejection {
    pub kind: PromptRejectionKind,
    pub message: String,
}

/// A completed Bas submission RPC. Transport failure is deliberately outside
/// this enum: it leaves acceptance unknown rather than implying rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptSubmissionOutcome {
    Accepted(TurnReceipt),
    Rejected(PromptRejection),
}
