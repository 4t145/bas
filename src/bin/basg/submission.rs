//! Prepare an immutable prompt once; title preflight is never part of a retry.
use crate::{
    attachment::Attachment,
    connection::{Failure, request},
};
use bas::{
    acp,
    model::*,
    service::{Client, pb},
};
use std::sync::Arc;

pub(crate) fn prepare(
    session: SessionId,
    turn: TurnId,
    text: &str,
    attachments: &[Attachment],
    capabilities: &acp::PromptCapabilities,
) -> Arc<PromptSubmission> {
    Arc::new(PromptSubmission {
        turn,
        request: acp::PromptRequest::new(
            session.to_string(),
            crate::attachment::prompt_content(text, attachments, capabilities),
        ),
    })
}

/// # Errors
/// Preflight failure is a definite local rejection; submit failure leaves
/// acceptance unknown. Both retain the original transport diagnostics.
pub(crate) async fn submit(
    client: &mut Client,
    session: SessionId,
    submission: &PromptSubmission,
    title: Option<String>,
) -> Result<PromptSubmissionOutcome, Failure> {
    if let Some(title) = title {
        request(async {
            client
                .set_title(pb::SetTitleRequest {
                    session: session.to_string(),
                    title,
                })
                .await?;
            Ok(())
        })
        .await
        .map_err(|failure| Failure::Preflight(Box::new(failure)))?;
    }
    request(async { Ok(client.submit_prompt(submission).await?) }).await
}

#[cfg(test)]
mod tests;
