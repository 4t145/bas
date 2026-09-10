use anyhow::{Context, Result};
use basr::service::{Client, pb};

/// Sends a user turn and prints the turn id.
///
/// # Errors
///
/// Returns an error when the daemon rejects the prompt.
pub(super) async fn send(client: &mut Client, session: &str, text: Vec<String>) -> Result<()> {
    let response = client
        .prompt(pb::PromptRequest {
            session: session.to_owned(),
            content: vec![pb::ContentBlock {
                block: Some(pb::content_block::Block::Text(text.join(" "))),
            }],
        })
        .await
        .context("prompt failed")?
        .into_inner();

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
        .cancel(pb::CancelRequest {
            session: session.to_owned(),
        })
        .await
        .context("cancel failed")?;

    println!("cancelled {session}");
    Ok(())
}
