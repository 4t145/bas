//! Human-readable rendering of daemon responses.
//!
//! Output is line-oriented so that `bast` stays usable in a `tty` and pipes
//! cleanly into other tools.

use std::io::Write;

use anyhow::{Context, Result};
use bas::{
    acp,
    model::{self, EventKind},
    service::pb,
};

/// Prints the configured agent backends.
pub fn agents(agents: &[pb::Agent]) {
    if agents.is_empty() {
        crate::safe::safe_println!("no agents configured");
        return;
    }

    for agent in agents {
        if agent.command.is_empty() {
            crate::safe::safe_println!("{} ({})", agent.id, agent.kind);
        } else {
            crate::safe::safe_println!("{} ({}) {}", agent.id, agent.kind, agent.command);
        }
    }
}

/// Prints one line per session.
///
/// # Errors
/// Returns an error if the daemon sends malformed metadata or output cannot be written.
pub fn sessions(sessions: &[pb::Session]) -> Result<()> {
    write_sessions(sessions, &mut std::io::stdout())
}

/// # Errors
/// Returns an error for malformed metadata or a failed output write.
fn write_sessions(sessions: &[pb::Session], stdout: &mut impl Write) -> Result<()> {
    if sessions.is_empty() {
        crate::safe::safe_writeln!(stdout, "no sessions")?;
        return Ok(());
    }

    for session in sessions {
        let snapshot = model::SessionSnapshot::try_from(session.clone())
            .context("malformed session from the daemon")?;
        let session = &snapshot.meta;
        crate::safe::safe_writeln!(
            stdout,
            "{}  {:<9} {:<10} {}{}",
            session.id,
            snapshot.status(),
            session.agent,
            session.title.as_deref().unwrap_or("-"),
            if session.archived { " [archived]" } else { "" }
        )?;
    }
    Ok(())
}

/// Prints a session's metadata, one field per line.
///
/// # Errors
/// Returns an error if the daemon sends malformed metadata or output cannot be written.
pub fn session(session: &pb::Session) -> Result<()> {
    write_session(session, &mut std::io::stdout())
}

/// # Errors
/// Returns an error for malformed metadata or a failed output write.
fn write_session(session: &pb::Session, stdout: &mut impl Write) -> Result<()> {
    let snapshot = model::SessionSnapshot::try_from(session.clone())
        .context("malformed session from the daemon")?;
    let session = &snapshot.meta;
    crate::safe::safe_writeln!(stdout, "id        {}", session.id)?;
    crate::safe::safe_writeln!(stdout, "agent     {}", session.agent)?;
    crate::safe::safe_writeln!(stdout, "status    {}", snapshot.status())?;
    if let Some(failure) = &snapshot.runtime.failure {
        crate::safe::safe_writeln!(
            stdout,
            "committed {} (revision {})",
            session.status,
            session.revision.raw()
        )?;
        crate::safe::safe_writeln!(stdout, "runtime   {}", snapshot.runtime.id)?;
        crate::safe::safe_writeln!(stdout, "failure   {}", failure)?;
    }
    crate::safe::safe_writeln!(stdout, "archived  {}", session.archived)?;
    crate::safe::safe_writeln!(
        stdout,
        "title     {}",
        session.title.as_deref().unwrap_or("-")
    )?;
    crate::safe::safe_writeln!(
        stdout,
        "cwd       {}",
        session
            .cwd
            .as_deref()
            .unwrap_or(std::path::Path::new("-"))
            .display()
    )?;
    crate::safe::safe_writeln!(
        stdout,
        "owner     {}@{}",
        session.owner.user,
        session.owner.host
    )?;
    crate::safe::safe_writeln!(stdout, "created   {}", session.created_at.millis())?;
    crate::safe::safe_writeln!(stdout, "updated   {}", session.updated_at.millis())?;
    Ok(())
}

/// Renders consecutive ACP chunks together while keeping user, thought and agent output distinct.
#[derive(Default)]
pub struct Timeline {
    role: Option<model::Role>,
    interruption: Option<(model::SessionId, String)>,
}

impl Timeline {
    /// # Errors
    /// Returns an error when the event cannot be decoded or stdout cannot be written.
    pub fn event(&mut self, event: &pb::Event) -> Result<()> {
        let decoded =
            model::Event::try_from(event.clone()).context("malformed event from the daemon")?;
        self.write_event(&decoded, &mut std::io::stdout())
    }

    /// # Errors
    /// Returns an error when the final newline cannot be written.
    pub fn finish(&mut self) -> Result<()> {
        self.end_line(&mut std::io::stdout())
    }

    /// # Errors
    /// Returns an error if the output sink cannot accept the final newline.
    fn end_line(&mut self, stdout: &mut impl Write) -> Result<()> {
        if self.role.take().is_some() {
            crate::safe::safe_writeln!(stdout)?;
        }
        Ok(())
    }

    /// # Errors
    /// Returns an error if event serialization, writing, or flushing the output fails.
    fn write_event(&mut self, decoded: &model::Event, stdout: &mut impl Write) -> Result<()> {
        let chunk = match &decoded.kind {
            EventKind::Acp(acp::SessionUpdate::UserMessageChunk(chunk)) => {
                Some((model::Role::User, chunk))
            }
            EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(chunk)) => {
                Some((model::Role::Agent, chunk))
            }
            EventKind::Acp(acp::SessionUpdate::AgentThoughtChunk(chunk)) => {
                Some((model::Role::Thought, chunk))
            }
            _ => None,
        };
        if let Some((role, chunk)) = chunk
            && let acp::ContentBlock::Text(text) = &chunk.content
        {
            if self.role != Some(role) {
                self.end_line(stdout)?;
                crate::safe::safe_write!(stdout, "{:>6} {:>7} ", decoded.cursor, role)?;
                self.role = Some(role);
            }
            crate::safe::safe_write!(stdout, "{}", text.text)?;
            return stdout.flush().context("cannot flush stdout");
        }
        self.end_line(stdout)?;

        match &decoded.kind {
            EventKind::Failed { message } => {
                if self.interruption.take().as_ref() == Some(&(decoded.session, message.clone())) {
                    return stdout.flush().context("cannot flush stdout");
                }
                crate::safe::safe_writeln!(stdout, "{:>6} ---- failed: {message}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::Acp(update) => match update {
                acp::SessionUpdate::AgentMessageChunk(chunk)
                | acp::SessionUpdate::AgentThoughtChunk(chunk)
                | acp::SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
                    acp::ContentBlock::Text(text) => {
                        crate::safe::safe_write!(stdout, "{}", text.text)?
                    }
                    acp::ContentBlock::ResourceLink(link) => {
                        crate::safe::safe_writeln!(stdout, "[{}]({})", link.name, link.uri)?
                    }
                    acp::ContentBlock::Image(image) => {
                        crate::safe::safe_writeln!(stdout, "[image: {}]", image.mime_type)?
                    }
                    acp::ContentBlock::Audio(audio) => {
                        crate::safe::safe_writeln!(stdout, "[audio: {}]", audio.mime_type)?
                    }
                    content => {
                        crate::safe::safe_writeln!(stdout, "{}", serde_json::to_string(content)?)?
                    }
                },
                acp::SessionUpdate::ToolCall(call) => crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} tool {} [{}]",
                    decoded.cursor,
                    call.title,
                    enum_name(&call.status)?
                )?,
                acp::SessionUpdate::ToolCallUpdate(call) => crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} tool {} [{}]",
                    decoded.cursor,
                    call.tool_call_id,
                    call.fields
                        .status
                        .as_ref()
                        .map(enum_name)
                        .transpose()?
                        .unwrap_or_else(|| "update".into())
                )?,
                update => crate::safe::safe_writeln!(stdout, "{}", serde_json::to_string(update)?)?,
            },
            EventKind::TurnStarted { turn } => {
                self.interruption = None;
                crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} ---- turn {turn} started",
                    decoded.cursor
                )
                .context("cannot write to stdout")?;
            }
            EventKind::TurnFinished { turn, result } => {
                let stop = match result {
                    Ok(response) => serde_json::to_value(response.stop_reason)?
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned(),
                    Err(error) => format!("agent error: {}", error.message),
                };
                crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} ---- turn {turn} finished ({stop})",
                    decoded.cursor
                )
                .context("cannot write to stdout")?;
            }
            EventKind::TurnInterrupted { turn, reason } => {
                use model::InterruptionReason;
                self.interruption = match reason {
                    InterruptionReason::BackendFailure { message }
                    | InterruptionReason::DispatchFailure { message } => {
                        Some((decoded.session, message.clone()))
                    }
                    _ => None,
                };
                let detail = match reason {
                    InterruptionReason::BackendFailure { message } => {
                        format!("backend failure: {message}")
                    }
                    InterruptionReason::DispatchFailure { message } => {
                        format!("dispatch failure: {message}")
                    }
                    InterruptionReason::SessionStopped => "session stopped".into(),
                    InterruptionReason::DaemonRestarted => "daemon restarted".into(),
                    InterruptionReason::CancelledBeforeDispatch => {
                        "cancelled before dispatch".into()
                    }
                };
                crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} ---- turn {turn} interrupted by Bas ({detail})",
                    decoded.cursor
                )
                .context("cannot write to stdout")?;
            }
            EventKind::TitleChanged { title } => {
                crate::safe::safe_writeln!(stdout, "{:>6} ---- title {title:?}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::StatusChanged { status } => {
                crate::safe::safe_writeln!(stdout, "{:>6} ---- status {status}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::ArchiveChanged { archived } => {
                crate::safe::safe_writeln!(
                    stdout,
                    "{:>6} ---- archived {archived}",
                    decoded.cursor
                )
                .context("cannot write to stdout")?;
            }
        }

        stdout.flush().context("cannot flush stdout")
    }
}

/// Prints a cursor hint after a history page.
pub fn next_cursor(next: Option<u64>) {
    if let Some(cursor) = next {
        crate::safe::safe_println!("-- more history: --after {cursor}");
    }
}

fn enum_name(value: &impl serde::Serialize) -> Result<String> {
    Ok(serde_json::to_value(value)?
        .as_str()
        .unwrap_or("unknown")
        .to_owned())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod archive_tests;

#[cfg(test)]
mod runtime_tests;
