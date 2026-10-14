use anyhow::{Context, Result};
use bas::{
    acp,
    model::{PromptSubmission, PromptSubmissionOutcome, SessionId, TurnId, TurnReceipt},
    service::Client,
};

/// Sends a user turn and prints the turn id.
///
/// # Errors
///
/// Returns an explicit rejection or an error leaving acceptance unknown.
pub(super) async fn send(
    client: &mut Client,
    server: std::net::SocketAddr,
    session: &str,
    turn: Option<TurnId>,
    text: Vec<String>,
) -> Result<()> {
    let submission = submission(session, turn, text);
    eprintln!(
        "{}",
        crate::safe::Safe(format_args!("submission {}", submission.turn))
    );
    let outcome = client
        .submit_prompt(&submission)
        .await
        .with_context(|| unknown_message(server, session, submission.turn))?;

    report_outcome(outcome, &mut std::io::stdout().lock())
}

fn report_outcome(
    outcome: PromptSubmissionOutcome,
    output: &mut impl std::io::Write,
) -> Result<()> {
    match outcome {
        PromptSubmissionOutcome::Accepted(receipt) => {
            crate::safe::safe_writeln!(output, "turn {}", receipt.turn)?;
            Ok(())
        }
        PromptSubmissionOutcome::Rejected(rejection) => {
            anyhow::bail!(
                "prompt rejected ({:?}): {}",
                rejection.kind,
                rejection.message
            )
        }
    }
}

fn submission(session: &str, turn: Option<TurnId>, text: Vec<String>) -> PromptSubmission {
    PromptSubmission {
        turn: turn.unwrap_or_default(),
        request: acp::PromptRequest::new(session.to_owned(), vec![text.join(" ").into()]),
    }
}

fn unknown_message(server: std::net::SocketAddr, session: &str, turn: TurnId) -> String {
    format!(
        "prompt acceptance unknown for submission {turn}; query with `bast --server {server} prompt-status {session} {turn}` \
         or explicitly retry with `bast --server {server} prompt {session} --submission {turn} <exact original text>`; \
         do not send the same request with a new id"
    )
}

/// Queries acceptance without resubmitting.
///
/// # Errors
///
/// Returns an error if the read fails; the submission's acceptance remains unknown.
pub(super) async fn status(client: &mut Client, session: SessionId, turn: TurnId) -> Result<()> {
    let receipt = client.prompt_acceptance(session, turn).await.context(
        "submission status query failed; acceptance remains unknown; no prompt was sent",
    )?;
    report_status(receipt, turn, &mut std::io::stdout().lock())
}

fn report_status(
    receipt: Option<TurnReceipt>,
    turn: TurnId,
    output: &mut impl std::io::Write,
) -> Result<()> {
    match receipt {
        Some(receipt) => crate::safe::safe_writeln!(output, "turn {}", receipt.turn)?,
        None => crate::safe::safe_writeln!(
            output,
            "submission {turn}: acceptance unknown; no acceptance record found in this query; \
             this is not a rejection and does not rule out later acceptance; no prompt was sent"
        )?,
    }
    Ok(())
}

/// Cancels the running turn.
///
/// # Errors
///
/// Returns an error when the daemon or the backend refuses to cancel.
pub(super) async fn cancel(client: &mut Client, session: &str) -> Result<()> {
    client
        .cancel_acp(&acp::CancelNotification::new(session.to_owned()))
        .await
        .context("cancel failed")?;

    crate::safe::safe_println!("cancelled {session}");
    Ok(())
}

#[cfg(test)]
mod tests;
