//! Human-readable rendering of daemon responses.
//!
//! Output is line-oriented so that `basrctl` stays usable in a `tty` and pipes
//! cleanly into other tools.

use std::io::Write;

use anyhow::{Context, Result};
use basr::{
    acp,
    model::{self, EventKind},
    service::pb,
};

/// Prints the configured agent backends.
pub fn agents(agents: &[pb::Agent]) {
    if agents.is_empty() {
        println!("no agents configured");
        return;
    }

    for agent in agents {
        if agent.command.is_empty() {
            println!("{} ({})", agent.id, agent.kind);
        } else {
            println!("{} ({}) {}", agent.id, agent.kind, agent.command);
        }
    }
}

/// Prints one line per session.
///
/// # Errors
/// Returns an error if the daemon sends malformed session metadata.
pub fn sessions(sessions: &[pb::Session]) -> Result<()> {
    if sessions.is_empty() {
        println!("no sessions");
        return Ok(());
    }

    for session in sessions {
        let session = model::SessionMeta::try_from(session.clone())
            .context("malformed session from the daemon")?;
        println!(
            "{}  {:<9} {:<10} {}",
            session.id,
            session.status,
            session.agent,
            session.title.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// Prints a session's metadata, one field per line.
///
/// # Errors
/// Returns an error if the daemon sends malformed session metadata.
pub fn session(session: &pb::Session) -> Result<()> {
    let session = model::SessionMeta::try_from(session.clone())
        .context("malformed session from the daemon")?;
    println!("id        {}", session.id);
    println!("agent     {}", session.agent);
    println!("status    {}", session.status);
    println!("title     {}", session.title.as_deref().unwrap_or("-"));
    println!(
        "cwd       {}",
        session
            .cwd
            .as_deref()
            .unwrap_or(std::path::Path::new("-"))
            .display()
    );
    println!("owner     {}@{}", session.owner.user, session.owner.host);
    println!("created   {}", session.created_at.millis());
    println!("updated   {}", session.updated_at.millis());
    Ok(())
}

/// Renders consecutive ACP chunks together while keeping user, thought and agent output distinct.
#[derive(Default)]
pub struct Timeline {
    role: Option<model::Role>,
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

    fn end_line(&mut self, stdout: &mut impl Write) -> Result<()> {
        if self.role.take().is_some() {
            writeln!(stdout)?;
        }
        Ok(())
    }

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
                write!(stdout, "{:>6} {:>7} ", decoded.cursor, role)?;
                self.role = Some(role);
            }
            write!(stdout, "{}", text.text)?;
            return stdout.flush().context("cannot flush stdout");
        }
        self.end_line(stdout)?;

        match &decoded.kind {
            EventKind::Failed { message } => {
                writeln!(stdout, "{:>6} ---- failed: {message}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::Acp(update) => match update {
                acp::SessionUpdate::AgentMessageChunk(chunk)
                | acp::SessionUpdate::AgentThoughtChunk(chunk)
                | acp::SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
                    acp::ContentBlock::Text(text) => write!(stdout, "{}", text.text)?,
                    acp::ContentBlock::ResourceLink(link) => {
                        writeln!(stdout, "[{}]({})", link.name, link.uri)?
                    }
                    acp::ContentBlock::Image(image) => {
                        writeln!(stdout, "[image: {}]", image.mime_type)?
                    }
                    acp::ContentBlock::Audio(audio) => {
                        writeln!(stdout, "[audio: {}]", audio.mime_type)?
                    }
                    content => writeln!(stdout, "{}", serde_json::to_string(content)?)?,
                },
                acp::SessionUpdate::ToolCall(call) => writeln!(
                    stdout,
                    "{:>6} tool {} [{}]",
                    decoded.cursor,
                    call.title,
                    enum_name(&call.status)?
                )?,
                acp::SessionUpdate::ToolCallUpdate(call) => writeln!(
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
                update => writeln!(stdout, "{}", serde_json::to_string(update)?)?,
            },
            EventKind::TurnStarted { turn } => {
                writeln!(stdout, "{:>6} ---- turn {turn} started", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::TurnFinished { turn, result } => {
                let stop = match result {
                    Ok(response) => serde_json::to_value(response.stop_reason)?
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned(),
                    Err(error) => format!("error: {error}"),
                };
                writeln!(
                    stdout,
                    "{:>6} ---- turn {turn} finished ({stop})",
                    decoded.cursor
                )
                .context("cannot write to stdout")?;
            }
            EventKind::TitleChanged { title } => {
                writeln!(stdout, "{:>6} ---- title {title:?}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
            EventKind::StatusChanged { status } => {
                writeln!(stdout, "{:>6} ---- status {status}", decoded.cursor)
                    .context("cannot write to stdout")?;
            }
        }

        stdout.flush().context("cannot flush stdout")
    }
}

/// Prints a cursor hint after a history page.
pub fn next_cursor(next: Option<u64>) {
    if let Some(cursor) = next {
        println!("-- more history: --after {cursor}");
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
