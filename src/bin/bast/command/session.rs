use anyhow::{Context, Result};
use bas::service::{Client, pb};

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
            clap::Session::List { all } => list(client, all).await,
            clap::Session::Show { session } => show(client, &session).await,
            clap::Session::Recover { session } => recover(client, &session).await,
            clap::Session::Archive { session } => set_archived(client, &session, true).await,
            clap::Session::Restore { session } => set_archived(client, &session, false).await,
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
    let cwd = protocol_cwd(&cwd)?;
    let owner = bas::model::Owner::current();
    let response = client
        .create_session(pb::CreateSessionRequest {
            agent,
            title,
            cwd: Some(cwd),
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

/// # Errors
/// Returns an error when listing fails or the returned metadata cannot be rendered.
async fn list(client: &mut Client, include_archived: bool) -> Result<()> {
    let response = client
        .list_sessions(pb::ListSessionsRequest { include_archived })
        .await
        .context("list sessions failed")?
        .into_inner();

    render::sessions(&response.sessions)
}

/// # Errors
/// Returns an actionable error for unsupported daemons, rejected calls, or malformed metadata.
async fn set_archived(client: &mut Client, session: &str, archived: bool) -> Result<()> {
    let response = client
        .set_archived(pb::SetArchivedRequest {
            session: session.to_owned(),
            archived,
        })
        .await
        .map_err(archive_error)?
        .into_inner();
    render::session(&archived_session(response)?)
}

fn archive_error(status: tonic::Status) -> anyhow::Error {
    if status.code() == tonic::Code::Unimplemented {
        anyhow::anyhow!(
            "daemon does not support recoverable archive/restore; upgrade bas and retry; \
             do not use session remove as a substitute (it permanently stops and forgets the session)"
        )
    } else {
        anyhow::Error::new(status).context("set session archive state failed")
    }
}

/// # Errors
/// Returns an error when the daemon omits the updated session.
fn archived_session(response: pb::SetArchivedResponse) -> Result<pb::Session> {
    response
        .session
        .context("set archive state response is missing the updated session")
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

async fn recover(client: &mut Client, session: &str) -> Result<()> {
    let response = client
        .restart_session(pb::GetSessionRequest {
            session: session.to_owned(),
        })
        .await
        .context(
            "session recovery failed; ensure storage has free space and upgrade bas if unsupported",
        )?
        .into_inner();
    render::session(
        &response
            .session
            .context("recovery response omitted the session")?,
    )
}

async fn remove(client: &mut Client, session: &str) -> Result<()> {
    client
        .delete_session(pb::DeleteSessionRequest {
            session: session.to_owned(),
        })
        .await
        .context("delete session failed")?;

    crate::safe::safe_println!("removed {session}");
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

    crate::safe::safe_println!("renamed {session}");
    Ok(())
}

/// # Errors
/// Returns an error if the path cannot be resolved or its canonical form is not UTF-8.
fn protocol_cwd(cwd: &std::path::Path) -> Result<String> {
    let cwd = std::fs::canonicalize(cwd).context("cannot resolve the working directory")?;
    Ok(bas::service::utf8_cwd(&cwd)?.to_owned())
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(test)]
mod archive_tests;
