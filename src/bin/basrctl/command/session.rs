use anyhow::{Context, Result};
use basr::service::{Client, pb};

use crate::{clap, render};

/// Runs a session subcommand.
///
/// # Errors
///
/// Returns an error when the daemon rejects the call.
pub(super) async fn run(client: &mut Client, command: clap::Session) -> Result<()> {
    match command {
        clap::Session::Create { agent, title, cwd } => create(client, agent, title, cwd).await,
        clap::Session::List => list(client).await,
        clap::Session::Show { session } => show(client, &session).await,
        clap::Session::Remove { session } => remove(client, &session).await,
        clap::Session::Rename { session, title } => rename(client, &session, &title).await,
    }
}

async fn create(
    client: &mut Client,
    agent: String,
    title: Option<String>,
    cwd: Option<std::path::PathBuf>,
) -> Result<()> {
    let response = client
        .create_session(pb::CreateSessionRequest {
            agent,
            title,
            cwd: cwd.map(|cwd| cwd.to_string_lossy().into_owned()),
            owner_user: None,
            owner_host: None,
        })
        .await
        .context("create session failed")?
        .into_inner();

    if let Some(session) = response.session {
        render::session(&session);
    }
    Ok(())
}

async fn list(client: &mut Client) -> Result<()> {
    let response = client
        .list_sessions(pb::ListSessionsRequest {})
        .await
        .context("list sessions failed")?
        .into_inner();

    render::sessions(&response.sessions);
    Ok(())
}

async fn show(client: &mut Client, session: &str) -> Result<()> {
    let response = client
        .get_session(pb::GetSessionRequest {
            session: session.to_owned(),
        })
        .await
        .context("get session failed")?
        .into_inner();

    if let Some(session) = response.session {
        render::session(&session);
    }
    Ok(())
}

async fn remove(client: &mut Client, session: &str) -> Result<()> {
    client
        .delete_session(pb::DeleteSessionRequest {
            session: session.to_owned(),
        })
        .await
        .context("delete session failed")?;

    println!("removed {session}");
    Ok(())
}

async fn rename(client: &mut Client, session: &str, title: &str) -> Result<()> {
    client
        .set_title(pb::SetTitleRequest {
            session: session.to_owned(),
            title: title.to_owned(),
        })
        .await
        .context("set title failed")?;

    println!("renamed {session}");
    Ok(())
}
