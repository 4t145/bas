use anyhow::{Context, Result};
use basr::{acp, service::Client};

/// Sends a user turn and prints the turn id.
///
/// # Errors
///
/// Returns an error when the daemon rejects the prompt.
pub(super) async fn send(client: &mut Client, session: &str, text: Vec<String>) -> Result<()> {
    let response = client
        .prompt_acp(&acp::PromptRequest::new(
            session.to_owned(),
            vec![text.join(" ").into()],
        ))
        .await
        .context("prompt failed")?;

    println!("turn {}", response.turn);
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

    println!("cancelled {session}");
    Ok(())
}
