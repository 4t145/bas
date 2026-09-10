//! Human-readable rendering of daemon responses.
//!
//! Output is line-oriented so that `basrctl` stays usable in a `tty` and pipes
//! cleanly into other tools.

use std::io::Write;

use anyhow::{Context, Result};
use basr::{
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
pub fn sessions(sessions: &[pb::Session]) {
    if sessions.is_empty() {
        println!("no sessions");
        return;
    }

    for session in sessions {
        println!("{}", session_line(session));
    }
}

/// Prints a session's metadata, one field per line.
pub fn session(session: &pb::Session) {
    println!("id        {}", session.id);
    println!("agent     {}", session.agent);
    println!("status    {}", status_name(session.status));
    println!("title     {}", session.title.as_deref().unwrap_or("-"));
    println!("cwd       {}", session.cwd.as_deref().unwrap_or("-"));
    if let Some(owner) = &session.owner {
        println!("owner     {}@{}", owner.user, owner.host);
    }
    println!("created   {}", session.created_at);
    println!("updated   {}", session.updated_at);
}

/// Prints an event, streaming deltas inline and everything else as its own
/// line.
pub fn event(event: &pb::Event) -> Result<()> {
    let decoded =
        model::Event::try_from(event.clone()).context("malformed event from the daemon")?;
    let mut stdout = std::io::stdout();

    match &decoded.kind {
        EventKind::Delta(delta) => {
            let text = match &delta.block {
                model::DeltaBlock::Text { text } | model::DeltaBlock::Thinking { text } => text,
            };
            write!(stdout, "{text}").context("cannot write to stdout")?;
        }
        EventKind::Message(message) => {
            let role = match message.role {
                model::Role::User => "user",
                model::Role::Agent => "agent",
            };
            writeln!(
                stdout,
                "{:>6} {role:>5} {}",
                decoded.cursor,
                message.plain_text()
            )
            .context("cannot write to stdout")?;
        }
        EventKind::ToolCall(call) => {
            writeln!(
                stdout,
                "{:>6} tool  {} [{}]",
                decoded.cursor,
                call.title,
                tool_status(call.status)
            )
            .context("cannot write to stdout")?;
        }
        EventKind::TurnStarted { turn } => {
            writeln!(stdout, "{:>6} ---- turn {turn} started", decoded.cursor)
                .context("cannot write to stdout")?;
        }
        EventKind::TurnFinished { turn, stop } => {
            writeln!(
                stdout,
                "{:>6} ---- turn {turn} finished ({})",
                decoded.cursor,
                stop_name(*stop)
            )
            .context("cannot write to stdout")?;
        }
        EventKind::TitleChanged { title } => {
            writeln!(stdout, "{:>6} ---- title {title:?}", decoded.cursor)
                .context("cannot write to stdout")?;
        }
        EventKind::StatusChanged { status } => {
            writeln!(
                stdout,
                "{:>6} ---- status {}",
                decoded.cursor,
                status_label(*status)
            )
            .context("cannot write to stdout")?;
        }
    }

    stdout.flush().context("cannot flush stdout")
}

/// Prints a cursor hint after a history page.
pub fn next_cursor(next: Option<u64>) {
    if let Some(cursor) = next {
        println!("-- more history: --after {cursor}");
    }
}

fn session_line(session: &pb::Session) -> String {
    format!(
        "{}  {:<9} {:<10} {}",
        session.id,
        status_name(session.status),
        session.agent,
        session.title.as_deref().unwrap_or("-")
    )
}

fn status_name(status: i32) -> &'static str {
    match pb::SessionStatus::try_from(status) {
        Ok(pb::SessionStatus::Starting) => "starting",
        Ok(pb::SessionStatus::Idle) => "idle",
        Ok(pb::SessionStatus::Working) => "working",
        Ok(pb::SessionStatus::Stopped) => "stopped",
        Ok(pb::SessionStatus::Failed) => "failed",
        Ok(pb::SessionStatus::Unspecified) | Err(_) => "unknown",
    }
}

fn status_label(status: model::SessionStatus) -> &'static str {
    match status {
        model::SessionStatus::Starting => "starting",
        model::SessionStatus::Idle => "idle",
        model::SessionStatus::Working => "working",
        model::SessionStatus::Stopped => "stopped",
        model::SessionStatus::Failed => "failed",
    }
}

fn stop_name(stop: model::StopReason) -> &'static str {
    match stop {
        model::StopReason::EndTurn => "end_turn",
        model::StopReason::MaxTokens => "max_tokens",
        model::StopReason::Cancelled => "cancelled",
        model::StopReason::Refusal => "refusal",
        model::StopReason::Other => "other",
    }
}

fn tool_status(status: model::ToolCallStatus) -> &'static str {
    match status {
        model::ToolCallStatus::Pending => "pending",
        model::ToolCallStatus::InProgress => "running",
        model::ToolCallStatus::Completed => "done",
        model::ToolCallStatus::Failed => "failed",
    }
}
