//! Runs only daemon-validated authentication plans in an explicit terminal.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Stdio,
};
const HELPER_FLAG: &str = "--basr-terminal-auth-helper";
const INVOCATION_ENV: &str = "_BASR_TERMINAL_AUTH_INVOCATION";
const RESULT_ENV: &str = "_BASR_TERMINAL_AUTH_RESULT";
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
const RESULT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const TERMINAL_RESULT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Invocation {
    #[serde(default)]
    language: crate::i18n::Language,
    program: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("{}", crate::i18n::tr!("auth-terminal-unchecked"))]
    MissingProgram,
    #[error("{}", crate::i18n::tr!("auth-terminal-invalid"))]
    InvalidCommand,
    #[error("{}", crate::i18n::tr!("auth-terminal-missing"))]
    NoTerminal,
    #[error("{}", crate::i18n::tr!("auth-terminal-start-failed"))]
    Launch,
    #[error("{}", crate::i18n::tr!("auth-program-start-failed", error = .0.to_string()))]
    LoginLaunch(std::io::Error),
}

impl Invocation {
    pub(crate) fn from_plan(plan: basr::authentication::Invocation) -> Result<Self, Error> {
        let invocation = Self {
            language: crate::i18n::language(),
            program: plan.program,
            args: plan.args,
            env: plan.env,
        };
        invocation.validate()?;
        Ok(invocation)
    }

    /// # Errors
    /// Rejects invalid argv/env and the reserved terminal-helper transport key.
    fn validate(&self) -> Result<(), Error> {
        if !self.program.is_absolute() {
            return Err(Error::MissingProgram);
        }
        if self.program.as_os_str().as_encoded_bytes().contains(&0)
            || self.args.iter().any(|arg| arg.contains('\0'))
            || self.env.iter().any(|(key, value)| {
                key.is_empty()
                    || key == INVOCATION_ENV
                    || key == RESULT_ENV
                    || key.contains(['=', '\0'])
                    || value.contains('\0')
            })
        {
            return Err(Error::InvalidCommand);
        }
        Ok(())
    }

    fn agent_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command
            .args(&self.args)
            .envs(&self.env)
            .env_remove(INVOCATION_ENV);
        command.env_remove(RESULT_ENV);
        command
    }
}

struct Terminal {
    program: PathBuf,
    prefix: &'static [&'static str],
}

/// # Errors
/// Reports that no supported terminal executable is available in the GUI environment.
fn terminal() -> Result<Terminal, Error> {
    [
        ("alacritty", &["-e"][..]),
        ("kitty", &[][..]),
        ("foot", &["-e"][..]),
        ("xterm", &["-e"][..]),
    ]
    .into_iter()
    .find_map(|(program, prefix)| {
        which::which(program)
            .ok()
            .map(|program| Terminal { program, prefix })
    })
    .ok_or(Error::NoTerminal)
}

/// # Errors
/// Reports unavailable terminal software or process launch failure, not auth completion.
pub(crate) fn launch(invocation: Invocation) -> Result<Login, Error> {
    let helper = std::env::current_exe().map_err(|_| Error::Launch)?;
    let directory = tempfile::tempdir().map_err(|_| Error::Launch)?;
    let result = directory.path().join("result");
    let child = terminal_command(&invocation, terminal()?, helper)?
        .env(RESULT_ENV, &result)
        .spawn()
        .map_err(|_| Error::Launch)?;
    Ok(Login {
        child,
        result,
        _directory: directory,
    })
}

pub(crate) struct Login {
    child: tokio::process::Child,
    result: PathBuf,
    _directory: tempfile::TempDir,
}

impl Login {
    #[cfg(test)]
    pub(crate) async fn complete(self) -> Result<(), String> {
        match self.complete_code().await? {
            0 => Ok(()),
            code => Err(crate::i18n::tr!(
                "auth-exit-failed",
                code = (code).to_string()
            )),
        }
    }

    /// # Errors
    /// Reports the helper's actual login failure or absence of completion feedback.
    pub(crate) async fn complete_code(self) -> Result<i32, String> {
        let mut child = self.child;
        let mut terminal = tokio::spawn(async move { child.wait().await });
        tokio::time::timeout(LOGIN_TIMEOUT, async {
            let mut terminal_exit: Option<(tokio::time::Instant, String)> = None;
            loop {
                if let Ok(value) = tokio::fs::read_to_string(&self.result).await
                    && let Ok(code) = value.trim().parse::<i32>()
                {
                    return Ok(code);
                }
                if let Some((closed_at, detail)) = &terminal_exit
                    && tokio::time::Instant::now().duration_since(*closed_at)
                        >= TERMINAL_RESULT_GRACE
                {
                    return Err(crate::i18n::tr!("auth-terminal-exited", detail = (detail).to_string()));
                }
                tokio::select! {
                    _ = tokio::time::sleep(RESULT_INTERVAL) => {},
                    result = &mut terminal, if terminal_exit.is_none() => {
                        let detail = match result {
                            Ok(Ok(status)) => status.code().map_or_else(
                                || crate::i18n::tr!("process-terminated").into(),
                                |code| crate::i18n::tr!("terminal-exit-code", code = (code).to_string()),
                            ),
                            Ok(Err(error)) => error.to_string(),
                            Err(_) => crate::i18n::tr!("process-exit-unavailable").into(),
                        };
                        terminal_exit = Some((tokio::time::Instant::now(), detail));
                    },
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(crate::i18n::tr!("auth-check-after-terminal").into()))
    }
}

/// # Errors
/// Rejects invalid invocations or unserializable helper payloads.
fn terminal_command(
    invocation: &Invocation,
    terminal: Terminal,
    helper: PathBuf,
) -> Result<tokio::process::Command, Error> {
    invocation.validate()?;
    let payload = serde_json::to_string(invocation).map_err(|_| Error::InvalidCommand)?;
    let mut command = tokio::process::Command::new(terminal.program);
    command
        .args(terminal.prefix)
        .arg(helper)
        .arg(HELPER_FLAG)
        // Do not apply agent-controlled PATH/loader/desktop variables to the emulator.
        .env(INVOCATION_ENV, payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    Ok(command)
}

/// Private helper mode: apply the method's environment only to the configured agent.
/// The payload stays in memory/environment, never argv or a credentials file.
pub(crate) fn run_helper_if_requested() {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(HELPER_FLAG)) {
        return;
    }
    let result_path = std::env::var_os(RESULT_ENV).map(PathBuf::from);
    let invocation = (|| -> Result<Invocation, Error> {
        let value = std::env::var(INVOCATION_ENV).map_err(|_| Error::InvalidCommand)?;
        let invocation: Invocation =
            serde_json::from_str(&value).map_err(|_| Error::InvalidCommand)?;
        crate::i18n::set_language(invocation.language);
        invocation.validate()?;
        Ok(invocation)
    })();
    std::process::exit(execute_helper(
        invocation,
        result_path.as_deref(),
        std::io::stdin().is_terminal(),
    ));
}

fn execute_helper(
    invocation: Result<Invocation, Error>,
    result_path: Option<&Path>,
    hold_failure: bool,
) -> i32 {
    let status = invocation.and_then(|invocation| {
        invocation
            .agent_command()
            .status()
            .map_err(Error::LoginLaunch)
    });
    let code = match status {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            let code = status.code().unwrap_or(1);
            eprintln!("{}", crate::i18n::tr!("auth-exit-failed", code = code));
            code
        }
        Err(error) => {
            eprintln!("\n{error}");
            1
        }
    };
    if let Some(path) = result_path {
        // Report before waiting for acknowledgement; the GUI need not wait for the terminal to close.
        let _ = publish_result(path, code);
    }
    if code != 0 && hold_failure {
        eprint!("{}", crate::i18n::tr!("auth-terminal-close"));
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
    }
    code
}

/// # Errors
/// Reports failure to atomically publish the helper's exit code.
fn publish_result(path: &Path, code: i32) -> std::io::Result<()> {
    let mut result = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    write!(result, "{code}")?;
    result
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests;
