//! Runs only daemon-validated authentication plans in an explicit terminal.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Write},
    path::PathBuf,
    process::Stdio,
};

mod control;
mod process;

const HELPER_FLAG: &str = "--basr-terminal-auth-helper";
const INVOCATION_ENV: &str = "_BASR_TERMINAL_AUTH_INVOCATION";
const CONTROL_ENV: &str = "_BASR_TERMINAL_AUTH_CONTROL";
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
const TERMINAL_RESULT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
const CANCELLED_EXIT_CODE: i32 = 130;

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
    #[error("{}", crate::i18n::tr!("auth-terminal-control-failed"))]
    Control(#[source] std::io::Error),
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
                    || key == CONTROL_ENV
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
        command.env_remove(CONTROL_ENV);
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
    let control = control::Listener::bind().map_err(Error::Control)?;
    let child = terminal_command(&invocation, terminal()?, helper)?
        .env(
            CONTROL_ENV,
            serde_json::to_string(control.endpoint()).map_err(|_| Error::InvalidCommand)?,
        )
        .spawn()
        .map_err(|_| Error::Launch)?;
    Ok(Login::new(child, control))
}

pub(crate) struct Login {
    child: tokio::process::Child,
    control: Option<control::Listener>,
}

impl Drop for Login {
    fn drop(&mut self) {
        if self.control.is_some() {
            // No helper has been authorised yet, so no login can be running.
            let _ = self.child.start_kill();
        }
    }
}

impl Login {
    fn new(child: tokio::process::Child, control: control::Listener) -> Self {
        Self {
            child,
            control: Some(control),
        }
    }

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
        self.complete_with_timeout(LOGIN_TIMEOUT).await
    }

    async fn complete_with_timeout(self, timeout: std::time::Duration) -> Result<i32, String> {
        tokio::time::timeout(timeout, self.wait())
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("auth-check-after-terminal").into()))
    }

    async fn wait(mut self) -> Result<i32, String> {
        let mut terminal_exit: Option<(tokio::time::Instant, String)> = None;
        let mut controller = {
            let accept = self
                .control
                .as_ref()
                .expect("listener retained until helper authentication")
                .accept();
            tokio::pin!(accept);
            loop {
                let deadline = terminal_exit
                    .as_ref()
                    .map(|(at, _)| *at + TERMINAL_RESULT_GRACE)
                    .unwrap_or_else(|| tokio::time::Instant::now() + LOGIN_TIMEOUT);
                tokio::select! {
                    result = &mut accept => break result.map_err(|error| Error::Control(error).to_string())?,
                    result = self.child.wait(), if terminal_exit.is_none() => {
                        let detail = match result {
                            Ok(status) => status.code().map_or_else(
                                || crate::i18n::tr!("process-terminated").into(),
                                |code| crate::i18n::tr!("terminal-exit-code", code = (code).to_string()),
                            ),
                            Err(error) => error.to_string(),
                        };
                        terminal_exit = Some((tokio::time::Instant::now(), detail));
                    },
                    _ = tokio::time::sleep_until(deadline), if terminal_exit.is_some() => {
                        let (_, detail) = terminal_exit.as_ref().expect("terminal exited");
                        return Err(crate::i18n::tr!("auth-terminal-exited", detail = detail.to_string()));
                    }
                }
            }
        };
        self.control.take();
        // The helper owns execution; the launcher may exit while the helper is
        // still active, or keep a completed failure window open for its user.
        let result = controller.result();
        tokio::pin!(result);
        let mut reaped = terminal_exit.is_some();
        loop {
            tokio::select! {
                result = &mut result => return result.map_err(|error| Error::Control(error).to_string()),
                _ = self.child.wait(), if !reaped => reaped = true,
            }
        }
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
    let endpoint = std::env::var(CONTROL_ENV)
        .map_err(|_| Error::InvalidCommand)
        .and_then(|value| {
            serde_json::from_str::<control::Endpoint>(&value).map_err(|_| Error::InvalidCommand)
        });
    let invocation = (|| -> Result<Invocation, Error> {
        let value = std::env::var(INVOCATION_ENV).map_err(|_| Error::InvalidCommand)?;
        let invocation: Invocation =
            serde_json::from_str(&value).map_err(|_| Error::InvalidCommand)?;
        crate::i18n::set_language(invocation.language);
        invocation.validate()?;
        Ok(invocation)
    })();
    let code = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(async {
            match endpoint {
                Ok(endpoint) => {
                    execute_helper(invocation, endpoint, std::io::stdin().is_terminal()).await
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }),
        Err(_) => {
            eprintln!("{}", crate::i18n::tr!("auth-terminal-start-failed"));
            1
        }
    };
    std::process::exit(code);
}

async fn execute_helper(
    invocation: Result<Invocation, Error>,
    endpoint: control::Endpoint,
    hold_failure: bool,
) -> i32 {
    let mut helper = match endpoint.connect().await {
        Ok(helper) => helper,
        Err(error) => {
            eprintln!("{}", Error::Control(error));
            return 1;
        }
    };
    let status = match invocation {
        Ok(invocation) => process::run(invocation.agent_command(), helper.cancelled())
            .await
            .map_err(Error::LoginLaunch),
        Err(error) => Err(error),
    };
    let code = match status {
        Ok(process::Outcome::Cancelled) => return CANCELLED_EXIT_CODE,
        Ok(process::Outcome::Exited(status)) if status.success() => 0,
        Ok(process::Outcome::Exited(status)) => {
            let code = status.code().unwrap_or(1);
            eprintln!("{}", crate::i18n::tr!("auth-exit-failed", code = code));
            code
        }
        Err(error) => {
            eprintln!("\n{error}");
            1
        }
    };
    // Completion transfers the failed window to the user; closing the control
    // connection afterwards must not terminate that completed presentation.
    let reported = helper.finish(code).await.is_ok();
    if code != 0 && hold_failure && reported {
        eprint!("{}", crate::i18n::tr!("auth-terminal-close"));
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
    }
    code
}

#[cfg(test)]
mod tests;
