use anyhow::{Context, Result};
use bas::service::{Client, pb};

use crate::render;
use crate::safe::safe_println;

/// Lists the daemon's configured agent backends.
///
/// # Errors
///
/// Returns an error when the daemon rejects the call.
pub(super) async fn list(client: &mut Client) -> Result<()> {
    let response = client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .context("list agents failed")?
        .into_inner();

    render::agents(&response.agents);
    Ok(())
}

/// # Errors
/// Reports unsupported daemons, acquisition failures, stale settings or failed login.
pub(super) async fn manage(
    command: crate::clap::Agent,
    client: &mut Client,
    server: &bas::service::Target,
) -> Result<()> {
    use crate::clap::Agent;
    match command {
        Agent::Catalog { refresh, json } => {
            let catalog = catalog(client, refresh).await?;
            if json {
                safe_println!("{}", serde_json::to_string_pretty(&catalog)?);
            } else {
                if let Some(warning) = catalog.warning {
                    safe_println!("{warning}");
                }
                safe_println!("ID\tNAME\tVERSION\tSTATUS");
                for entry in catalog.entries {
                    safe_println!(
                        "{}\t{}\t{}\t{}",
                        entry.id,
                        entry.name,
                        entry.installed_version.as_deref().unwrap_or(&entry.version),
                        status(entry.status)
                    );
                }
            }
        }
        Agent::Install { id } => {
            let mut stream = client
                .install_agent(pb::InstallAgentRequest { id: id.clone() })
                .await?
                .into_inner();
            let mut complete = false;
            while let Some(update) = stream.message().await? {
                safe_println!("{}: {}", update.phase, update.message);
                complete |= update.complete;
            }
            anyhow::ensure!(complete, "Installation ended before registration completed");
            let spec = spec(client, &id).await?;
            let checked = check(client, &spec).await?;
            show_check(&id, &checked);
        }
        Agent::Check { id } => {
            let spec = spec(client, &id).await?;
            let checked = check(client, &spec).await?;
            show_check(&id, &checked);
            anyhow::ensure!(
                checked.status() == pb::AgentCheckStatus::Connected,
                "Agent is not ready to create sessions"
            );
        }
        Agent::Login { id, method } => {
            let spec = spec(client, &id).await?;
            login(client, &spec, method.as_deref(), server).await?;
        }
        Agent::Remove { id } => {
            client
                .remove_agent(pb::RemoveAgentRequest { id: id.clone() })
                .await?;
            safe_println!("Removed {id}; credentials and version directories retained");
        }
    }
    Ok(())
}

fn status(status: bas::management::Status) -> &'static str {
    use bas::management::Status;
    match status {
        Status::NotInstalled => "not installed",
        Status::Installed => "installed; readiness unchecked",
        Status::Missing => "executable missing",
        Status::Unsupported => "unsupported distribution",
    }
}

fn require_local_terminal(server: &bas::service::Target) -> Result<()> {
    anyhow::ensure!(
        server.is_local(),
        "Complete terminal authentication on the server host"
    );
    Ok(())
}

async fn catalog(client: &mut Client, refresh: bool) -> Result<bas::management::Catalog> {
    let response = client
        .list_agent_catalog(pb::ListAgentCatalogRequest { refresh })
        .await
        .context("agent catalog unavailable; upgrade bas if this method is unsupported")?
        .into_inner();
    serde_json::from_slice(&response.catalog_json).context("invalid agent catalog")
}

async fn spec(client: &mut Client, id: &str) -> Result<bas::config::AgentSpec> {
    catalog(client, false)
        .await?
        .entries
        .into_iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.spec)
        .ok_or_else(|| anyhow::anyhow!("Agent {id} is not configured; run bast agent install {id}"))
}

async fn check(
    client: &mut Client,
    spec: &bas::config::AgentSpec,
) -> Result<pb::CheckAgentResponse> {
    Ok(client
        .check_agent(pb::CheckAgentRequest {
            agent: spec.id.to_string(),
            expected_spec_json: serde_json::to_vec(spec)?,
            connect: true,
        })
        .await?
        .into_inner())
}

fn show_check(id: &str, checked: &pb::CheckAgentResponse) {
    safe_println!("{id}: {}", checked.message);
    if checked.status() == pb::AgentCheckStatus::AuthenticationRequired {
        safe_println!("Sign in with: bast agent login {id}");
    }
}

async fn login(
    client: &mut Client,
    spec: &bas::config::AgentSpec,
    selected: Option<&str>,
    server: &bas::service::Target,
) -> Result<()> {
    let checked = check(client, spec).await?;
    if checked.status() == pb::AgentCheckStatus::Connected {
        show_check(spec.id.as_str(), &checked);
        return Ok(());
    }
    anyhow::ensure!(
        checked.status() == pb::AgentCheckStatus::AuthenticationRequired,
        "Cannot complete agent setup: {}",
        checked.message
    );
    let initialization: serde_json::Value = checked
        .initialization
        .as_ref()
        .context("Agent did not advertise authentication")?
        .decode()?;
    let error: Option<serde_json::Value> = checked
        .authentication_error
        .as_ref()
        .map(pb::AcpPayload::decode)
        .transpose()?;
    let methods = bas::authentication::methods(&initialization, error.as_ref())?;
    let method = match selected {
        Some(id) => methods
            .iter()
            .find(|method| method.id == id)
            .context("Selected method was not advertised by the agent")?,
        None if methods.len() == 1 => &methods[0],
        None => {
            for method in &methods {
                safe_println!("{}\t{}", method.id, method.name);
            }
            anyhow::bail!("Choose an advertised method with --method <id>");
        }
    };
    if let bas::authentication::Execution::Unsupported { reason } = &method.execution {
        anyhow::bail!("{}: {reason}", method.name);
    }
    safe_println!("{}", method.name);
    if let Some(description) = &method.description {
        safe_println!("{description}");
    }
    let (controls, input) = tokio::sync::mpsc::channel(8);
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::Start(
                pb::AuthenticateAgentRequest {
                    agent: spec.id.to_string(),
                    expected_spec_json: serde_json::to_vec(spec)?,
                    method_id: method.id.clone(),
                    expected_method_json: serde_json::to_vec(method)?,
                },
            )),
        })
        .await
        .context("Authentication input closed")?;
    let mut output = client
        .authenticate_agent_interactive(tokio_stream::wrappers::ReceiverStream::new(input))
        .await
        .context("Interactive authentication unavailable; upgrade bas")?
        .into_inner();
    let mut queued = std::collections::VecDeque::new();
    loop {
        let event = if let Some(event) = queued.pop_front() {
            Some(event)
        } else {
            output.message().await?
        };
        let Some(event) = event else {
            break;
        };
        match event.kind {
            Some(pb::authentication_event::Kind::Check(ready)) => {
                show_check(spec.id.as_str(), &ready);
                anyhow::ensure!(
                    ready.status() == pb::AgentCheckStatus::Connected,
                    "Agent setup is incomplete; review its native configuration"
                );
                return Ok(());
            }
            Some(pb::authentication_event::Kind::Terminal(payload)) => {
                require_local_terminal(server)?;
                let invocation: bas::authentication::Invocation = payload.decode()?;
                use std::io::IsTerminal;
                anyhow::ensure!(
                    std::io::stdin().is_terminal(),
                    "This setup method requires an interactive terminal"
                );
                let status = tokio::process::Command::new(&invocation.program)
                    .args(&invocation.args)
                    .envs(&invocation.env)
                    .stdin(std::process::Stdio::inherit())
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .kill_on_drop(true)
                    .status()
                    .await
                    .context("Could not start the agent's setup terminal")?;
                controls
                    .send(pb::AuthenticationInput {
                        kind: Some(pb::authentication_input::Kind::TerminalResult(
                            pb::TerminalAuthenticationResult {
                                exit_code: status.code().unwrap_or(-1),
                            },
                        )),
                    })
                    .await
                    .context("Authentication input closed")?;
            }
            Some(pb::authentication_event::Kind::Elicitation(pending)) => {
                let raw: serde_json::Value = pending
                    .request
                    .context("Missing elicitation request")?
                    .decode()?;
                let response = elicitation_response(&raw);
                tokio::pin!(response);
                let response = loop {
                    tokio::select! {
                        result = &mut response => break Some(result?),
                        incoming = output.message() => {
                            let incoming = incoming?.context("Authentication ended while waiting for input")?;
                            match incoming.kind {
                                Some(pb::authentication_event::Kind::Check(ready)) => {
                                    show_check(spec.id.as_str(), &ready);
                                    anyhow::ensure!(ready.status() == pb::AgentCheckStatus::Connected,
                                        "Agent setup is incomplete; review its native configuration");
                                    return Ok(());
                                }
                                Some(pb::authentication_event::Kind::Progress(message)) => safe_println!("{message}"),
                                Some(pb::authentication_event::Kind::Elicitation(_)) => queued.push_back(incoming),
                                Some(pb::authentication_event::Kind::ElicitationComplete(_)) => queued.push_back(incoming),
                                Some(pb::authentication_event::Kind::ElicitationCancelled(ref id)) => {
                                    if id == &pending.request_id {
                                        safe_println!("Agent cancelled this input request");
                                        break None;
                                    }
                                    queued.push_back(incoming);
                                }
                                _ => anyhow::bail!("Agent changed its setup flow while waiting for input"),
                            }
                        }
                    }
                };
                let Some(response) = response else {
                    continue;
                };
                controls
                    .send(pb::AuthenticationInput {
                        kind: Some(pb::authentication_input::Kind::ElicitationResponse(
                            pb::ElicitationAnswer {
                                request_id: pending.request_id,
                                response: Some((&response).into()),
                            },
                        )),
                    })
                    .await
                    .context("Authentication input closed")?;
            }
            Some(pb::authentication_event::Kind::ElicitationComplete(payload)) => {
                let raw: serde_json::Value = payload.decode()?;
                safe_println!(
                    "Authorization completed: {}",
                    raw.get("elicitationId")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                );
            }
            Some(pb::authentication_event::Kind::Progress(message)) => safe_println!("{message}"),
            Some(pb::authentication_event::Kind::ElicitationCancelled(_)) => {}
            None => anyhow::bail!("Missing authentication event"),
        }
    }
    anyhow::bail!("Authentication ended before the readiness check")
}

/// # Errors
/// Reports unsupported schemas or failure reading an interactive response.
async fn elicitation_response(raw: &serde_json::Value) -> Result<serde_json::Value> {
    use bas::authentication::elicitation::Mode;
    use std::io::IsTerminal;
    let request = bas::authentication::elicitation::parse_request(raw)?;
    if !std::io::stdin().is_terminal() {
        safe_println!("Interactive agent setup requires a terminal");
        return Ok(serde_json::json!({"action":"cancel"}));
    }
    safe_println!("{}", request.message);
    match &request.mode {
        Mode::Url(url) => {
            safe_println!("Open this address in your browser: {}", url.url);
            action_prompt(None).await
        }
        Mode::Form(form) => {
            let mut content = form.defaults();
            for field in &form.fields {
                loop {
                    safe_println!(
                        "{}{}",
                        field.title,
                        if field.required {
                            " (required)"
                        } else {
                            " (optional)"
                        }
                    );
                    if let Some(description) = &field.description {
                        safe_println!("{description}");
                    }
                    describe_field(field);
                    if let Some(default) = content.get(&field.name) {
                        safe_println!("Default: {default}");
                    }
                    let line = read_line().await?;
                    let parsed = match parse_input(field, &line) {
                        Ok(value) => value,
                        Err(error) => {
                            safe_println!("{error}");
                            continue;
                        }
                    };
                    let Some(value) = parsed else {
                        if !field.required || content.contains_key(&field.name) {
                            break;
                        }
                        safe_println!("A value is required");
                        continue;
                    };
                    if let Err(error) = field.validate(&value) {
                        safe_println!("{error}");
                        continue;
                    }
                    content.insert(field.name.clone(), value);
                    break;
                }
            }
            form.validate(&content)?;
            safe_println!("{}", serde_json::to_string_pretty(&content)?);
            action_prompt(Some(content)).await
        }
    }
}

fn describe_field(field: &bas::authentication::elicitation::Field) {
    use bas::authentication::elicitation::Kind;
    match &field.kind {
        Kind::Enum { options } | Kind::MultiSelect { options } => {
            for (index, choice) in options.iter().enumerate() {
                safe_println!("{}: {}", index + 1, choice.title);
            }
            if matches!(field.kind, Kind::MultiSelect { .. }) {
                safe_println!("Enter choice numbers separated by commas");
            }
        }
        Kind::Boolean => safe_println!("Enter true or false"),
        Kind::Number | Kind::Integer => safe_println!("Enter a number"),
        Kind::String => safe_println!("Enter text"),
    }
}

/// # Errors
/// Reports a value that cannot be parsed as the schema's primitive type.
fn parse_input(
    field: &bas::authentication::elicitation::Field,
    line: &str,
) -> Result<Option<serde_json::Value>> {
    use bas::authentication::elicitation::Kind;
    use serde_json::Value;
    if line.is_empty() {
        return Ok(None);
    }
    let choice =
        |options: &[bas::authentication::elicitation::Choice], input: &str| -> Result<Value> {
            let index = input
                .trim()
                .parse::<usize>()
                .context("Enter an offered choice number")?;
            let item = index
                .checked_sub(1)
                .and_then(|index| options.get(index))
                .context("Choice number was not offered")?;
            Ok(Value::String(item.value.clone()))
        };
    let value = match &field.kind {
        Kind::String => Value::String(line.into()),
        Kind::Number | Kind::Integer | Kind::Boolean => {
            serde_json::from_str(line).context("Enter a valid number or boolean")?
        }
        Kind::Enum { options } => choice(options, line)?,
        Kind::MultiSelect { options } => Value::Array(
            line.split(',')
                .map(|input| choice(options, input))
                .collect::<Result<Vec<_>>>()?,
        ),
    };
    Ok(Some(value))
}

/// # Errors
/// Reports an input failure while the user selects submit, decline or cancel.
async fn action_prompt(
    content: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<serde_json::Value> {
    loop {
        safe_println!("[a] Accept and continue / [d] Decline / [c] Cancel");
        let action = match read_line().await?.trim().to_ascii_lowercase().as_str() {
            "a" | "accept" => "accept",
            "d" | "decline" => "decline",
            "c" | "cancel" | "" => "cancel",
            _ => continue,
        };
        let response = if action == "accept" && content.is_some() {
            serde_json::json!({"action":action,"content":content})
        } else {
            serde_json::json!({"action":action})
        };
        return Ok(response);
    }
}

/// Read on a process-owned thread so stdin never blocks Tokio's runtime.
///
/// # Errors
/// Reports EOF, thread creation failure or a failed terminal read.
async fn read_line() -> Result<String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("bast-elicitation-input".into())
        .spawn(move || {
            let result = terminal_line(&send);
            let _ = send.send(result);
        })?;
    receive
        .await
        .context("Terminal reader closed")?
        .map_err(Into::into)
}

/// # Errors
/// Reports cancellation, EOF, invalid UTF-8 or a failed terminal read.
#[cfg(unix)]
fn terminal_line(
    send: &tokio::sync::oneshot::Sender<std::io::Result<String>>,
) -> std::io::Result<String> {
    use nix::poll::{PollFd, PollFlags};
    use std::os::fd::AsFd;
    const INPUT_POLL_MS: u16 = 100;
    let stdin = std::io::stdin();
    let mut bytes = Vec::new();
    loop {
        if send.is_closed() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Input request cancelled",
            ));
        }
        let mut descriptors = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        if nix::poll::poll(&mut descriptors, INPUT_POLL_MS)? == 0 {
            continue;
        }
        if send.is_closed() {
            continue;
        }
        let mut byte = [0];
        if nix::unistd::read(&stdin, &mut byte)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Terminal input closed",
            ));
        }
        if byte[0] == b'\n' {
            return String::from_utf8(bytes)
                .map(|line| line.trim_end_matches('\r').to_owned())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        }
        bytes.push(byte[0]);
    }
}

/// # Errors
/// Reports EOF or a failed terminal read.
#[cfg(not(unix))]
fn terminal_line(
    _send: &tokio::sync::oneshot::Sender<std::io::Result<String>>,
) -> std::io::Result<String> {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "Terminal input closed",
        ));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests;
