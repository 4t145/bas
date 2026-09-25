//! Explicit, local terminal authentication using ACP's standard terminal method.
//!
//! The executable always comes from the checked agent configuration. Arbitrary
//! commands in `_meta` are not interpreted, and no command is passed through a shell.

use std::{collections::BTreeMap, path::PathBuf, process::Stdio};

use basr::{acp, config::AgentSpec};
use gpui_kit::component::*;
use gpui_kit::{App, Context, IntoElement, ParentElement, Styled, TestSupportExt, div, prelude::*};
use serde::{Deserialize, Serialize};

use super::{Agents, check};

impl Agents {
    /// # Errors
    /// Rejects edits in progress, changed saved specs and methods not in the checked response.
    fn authentication_invocation(
        &self,
        spec: &AgentSpec,
        method: &acp::AuthMethod,
        cx: &App,
    ) -> Result<Invocation, String> {
        if self.dirty(cx) {
            return Err("请先结束当前编辑。".into());
        }
        let saved = self
            .store
            .load()
            .map_err(|_| "无法核对当前配置；请重新加载。")?;
        if !saved.settings.daemon.agents.contains(spec) {
            return Err("Agent 配置已变化；请重新加载和连接后再认证。".into());
        }
        let Some(check::Status::AuthenticationRequired { program, methods }) = self.status(spec)
        else {
            return Err("认证状态已变化，请重新连接。".into());
        };
        if !methods.contains(method) {
            return Err("该认证方式已失效，请重新连接。".into());
        }
        Invocation::new(spec, program, method).map_err(|error| error.to_string())
    }

    fn confirm_authentication(
        &mut self,
        spec: AgentSpec,
        method: acp::AuthMethod,
        cx: &mut Context<Self>,
    ) {
        let invocation = match self.authentication_invocation(&spec, &method, cx) {
            Ok(invocation) => invocation,
            Err(message) => {
                self.message = message;
                cx.notify();
                return;
            }
        };
        let view = cx.entity().downgrade();
        if let Err(error) = crate::application::open_confirmation(
            crate::application::Confirmation {
                title: format!("认证 {}", spec.id),
                message: invocation.confirmation(),
                confirm_label: "打开认证终端".into(),
            },
            move |cx| {
                let _ = view.update(cx, |this, cx| this.authenticate(spec, method, cx));
            },
            cx,
        ) {
            self.message = format!("无法打开认证确认：{error}");
            cx.notify();
        }
    }

    fn authenticate(&mut self, spec: AgentSpec, method: acp::AuthMethod, cx: &mut Context<Self>) {
        // Revalidate after confirmation: external edits may have happened while
        // the confirmation window was open.
        let invocation = match self.authentication_invocation(&spec, &method, cx) {
            Ok(invocation) => invocation,
            Err(message) => {
                self.message = message;
                cx.notify();
                return;
            }
        };
        let Some(service) = cx.try_global::<check::Service>().cloned() else {
            self.message = "认证服务未启用，请重新打开 BASR。".into();
            cx.notify();
            return;
        };
        // Validation and spawn run in the same confirmation callback. Only after
        // launch does the terminal become an independently user-owned window.
        self.message = match service.authenticate(invocation) {
            Ok(()) => "认证终端已打开。请完成登录 / 模型配置后点击“连接”重新检测。关闭设置不会关闭外部终端。".into(),
            Err(message) => message,
        };
        cx.notify();
    }

    pub(super) fn authentication_content(
        &self,
        spec: &AgentSpec,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(check::Status::AuthenticationRequired { methods, .. }) = self.status(spec) else {
            return div().into_any_element();
        };
        div()
            .id(format!("agents-authentication-{index}"))
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .text_sm()
            .child("ACP 初始化已成功；Agent 要求登录或配置模型提供方后才能创建会话。")
            .when(methods.is_empty(), |panel| panel.child("Agent 未提供认证方式，请参考它的安装 / 登录文档。"))
            .children(methods.iter().enumerate().map(|(method_index, method)| {
                let terminal = matches!(method, acp::AuthMethod::Terminal(_));
                let spec = spec.clone();
                let selected = method.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(method.name().to_owned())
                    .when_some(method.description(), |row, description| {
                        row.child(div().text_color(cx.theme().muted_foreground).child(description.to_owned()))
                    })
                    .child(if terminal {
                        div().flex().child(
                            super::button(format!("agents-authenticate-{index}-{method_index}"))
                                .label("在终端中认证…")
                                .disabled(self.dirty(cx))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_authentication(spec.clone(), selected.clone(), cx);
                                })),
                        ).into_any_element()
                    } else {
                        div().text_color(cx.theme().muted_foreground)
                            .child("此方式需 Agent 主导的 authenticate 流程，目前未接入。请按该 Agent 文档登录后重新连接。")
                            .into_any_element()
                    })
            }))
            .into_any_element()
    }
}

const HELPER_FLAG: &str = "--basr-terminal-auth-helper";
const INVOCATION_ENV: &str = "_BASR_TERMINAL_AUTH_INVOCATION";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Invocation {
    program: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("认证方法不是终端认证；请按 Agent 的说明登录后重新连接。")]
    Unsupported,
    #[error("没有经过检测的本机可执行文件，请重新连接。")]
    MissingProgram,
    #[error("认证命令包含无效的参数或环境变量。")]
    InvalidCommand,
    #[error("未找到可用终端。请安装 Alacritty、Kitty、Foot 或 XTerm，再重试。")]
    NoTerminal,
    #[error("无法启动认证终端，请检查终端安装和图形会话。")]
    Launch,
}

impl Invocation {
    /// # Errors
    /// Rejects nonterminal methods, unchecked executable paths and invalid argv/env.
    pub(super) fn new(
        spec: &AgentSpec,
        program: &str,
        method: &acp::AuthMethod,
    ) -> Result<Self, Error> {
        let acp::AuthMethod::Terminal(method) = method else {
            return Err(Error::Unsupported);
        };
        let base = spec.command.as_ref().ok_or(Error::MissingProgram)?;
        let program = PathBuf::from(program);
        if !program.is_absolute() {
            return Err(Error::MissingProgram);
        }
        let mut args = base.args.clone();
        args.extend(method.args.clone());
        let mut env = spec.env.clone();
        env.extend(method.env.clone());
        let invocation = Self { program, args, env };
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
        command
    }

    /// Does not include environment values, which can contain credentials.
    pub(super) fn confirmation(&self) -> String {
        format!(
            "将在本机终端中运行这个 Agent 的认证命令：\n\n程序：{}\n参数：{}\n\n\
             由 Agent 提供登录或模型提供方设置界面。BASR 不收集密码或 API key。\n\
             这是独立的终端窗口，关闭设置不会关闭它。完成后回到设置点击“连接”重新检测。",
            self.program.display(),
            serde_json::to_string(&self.args).expect("string arguments serialize"),
        )
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
pub(super) fn launch(invocation: Invocation) -> Result<tokio::process::Child, Error> {
    let helper = std::env::current_exe().map_err(|_| Error::Launch)?;
    terminal_command(&invocation, terminal()?, helper)?
        .spawn()
        .map_err(|_| Error::Launch)
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
    let result = (|| -> Result<(), Error> {
        let value = std::env::var(INVOCATION_ENV).map_err(|_| Error::InvalidCommand)?;
        let invocation: Invocation =
            serde_json::from_str(&value).map_err(|_| Error::InvalidCommand)?;
        invocation.validate()?;
        let mut command = invocation.agent_command();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let _ = command.exec();
            Err(Error::Launch)
        }
        #[cfg(not(unix))]
        {
            let status = command.status().map_err(|_| Error::Launch)?;
            std::process::exit(status.code().unwrap_or(1))
        }
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
