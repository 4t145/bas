use anyhow::{Context, Result};
use basr::service::{Client, pb};

use crate::{clap, render};

impl clap::Session {
    /// Runs a session subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error when the daemon rejects the call.
    pub(crate) async fn run(self, client: &mut Client) -> Result<()> {
        match self {
            clap::Session::Create { agent, title, cwd } => create(client, agent, title, cwd).await,
            clap::Session::List => list(client).await,
            clap::Session::Show { session } => show(client, &session).await,
            clap::Session::Remove { session } => remove(client, &session).await,
            clap::Session::Rename { session, title } => rename(client, &session, &title).await,
        }
    }
}

async fn create(
    client: &mut Client,
    agent: String,
    title: Option<String>,
    cwd: Option<std::path::PathBuf>,
) -> Result<()> {
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir().context("cannot read the working directory")?,
    };
    let cwd = std::fs::canonicalize(cwd).context("cannot resolve the working directory")?;
    let owner = basr::model::Owner::current();
    let response = client
        .create_session(pb::CreateSessionRequest {
            agent,
            title,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            owner_user: Some(owner.user),
            owner_host: Some(owner.host),
        })
        .await
        .context("create session failed")?
        .into_inner();

    if let Some(session) = response.session {
        render::session(&session)?;
    }
    Ok(())
}

async fn list(client: &mut Client) -> Result<()> {
    let response = client
        .list_sessions(pb::ListSessionsRequest {})
        .await
        .context("list sessions failed")?
        .into_inner();

    render::sessions(&response.sessions)
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
        render::session(&session)?;
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
