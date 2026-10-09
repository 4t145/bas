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
    program: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("没有经过检测的本机可执行文件，请重新检测。")]
    MissingProgram,
    #[error("认证命令包含无效的参数或环境变量。")]
    InvalidCommand,
    #[error("未找到可用终端。请安装 Alacritty、Kitty、Foot 或 XTerm，再重试。")]
    NoTerminal,
    #[error("无法启动认证终端，请检查终端安装和图形会话。")]
    Launch,
    #[error("无法启动登录程序：{0}")]
    LoginLaunch(std::io::Error),
}

impl Invocation {
    pub(crate) fn from_plan(plan: basr::authentication::Invocation) -> Result<Self, Error> {
        let invocation = Self {
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
            code => Err(format!(
                "登录失败（退出码 {code}）。请查看登录终端中的错误信息后重试。"
            )),
        }
    }

    /// # Errors
    /// Reports the helper's actual login failure or absence of completion feedback.
    pub(crate) async fn complete_code(self) -> Result<i32, String> {
        let mut child = self.child;
        let mut terminal = tokio::spawn(async move { child.wait().await });
        tokio::time::timeout(LOGIN_TIMEOUT, async {
            let mut terminal_exit = None;
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
                    return Err(format!("登录终端已退出（{detail}）。请重新打开登录终端。"));
                }
                tokio::select! {
                    _ = tokio::time::sleep(RESULT_INTERVAL) => {},
                    result = &mut terminal, if terminal_exit.is_none() => {
                        let detail = match result {
                            Ok(Ok(status)) => status.code().map_or_else(
                                || "进程被中止".into(),
                                |code| format!("退出码 {code}"),
                            ),
                            Ok(Err(error)) => error.to_string(),
                            Err(_) => "无法读取退出结果".into(),
                        };
                        terminal_exit = Some((tokio::time::Instant::now(), detail));
                    },
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err("未收到登录完成结果。可完成终端登录后点击“重新检测”。".into()))
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
            eprintln!("\n登录失败（退出码 {code}）。请查看上方错误信息后重试。");
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
        eprint!("按 Enter 关闭登录终端…");
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
