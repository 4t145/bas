//! The local daemon, not the GUI's PATH, determines backend availability.

use std::{net::SocketAddr, time::Duration};

use basr::{
    config::AgentSpec,
    service::{Client, pb},
};
use gpui_kit::{App, Global};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

use super::authentication;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);
const RELOAD_ATTEMPTS: usize = 6;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct Service {
    server: SocketAddr,
    runtime: Handle,
}

impl Global for Service {}

pub(crate) fn init(server: SocketAddr, runtime: Handle, cx: &mut App) {
    cx.set_global(Service { server, runtime });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Checking,
    Connecting,
    Installed(String),
    Connected(String),
    AuthenticationRequired {
        program: String,
        methods: Vec<basr::acp::AuthMethod>,
    },
    Missing(String),
    Failed(String),
}

impl Status {
    pub(super) fn connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }

    pub(super) fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Connecting)
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Checking => "正在检测",
            Self::Connecting => "正在连接",
            Self::Installed(_) => "已检测到 · 待连接",
            Self::Connected(_) => "已连通",
            Self::AuthenticationRequired { .. } => "ACP 已连接 · 需要认证",
            Self::Missing(_) => "未找到可执行文件",
            Self::Failed(_) => "未连通",
        }
    }
}

impl Service {
    /// # Errors
    /// Rejects remote endpoints and terminal launch failures; does not report login success.
    pub(super) fn authenticate(
        &self,
        invocation: authentication::Invocation,
    ) -> Result<(), String> {
        if !self.server.ip().is_loopback() {
            return Err("远端 Agent 不能在本机认证，请在 daemon 所在主机完成登录。".into());
        }
        let _entered = self.runtime.enter();
        let mut child = authentication::launch(invocation).map_err(|error| error.to_string())?;
        // The visible terminal is now user-owned. Reap it, but don't kill it on
        // settings closure or mistake a launcher exit for successful authentication.
        self.runtime.spawn(async move {
            let _ = child.wait().await;
        });
        Ok(())
    }

    pub(super) fn request(
        &self,
        spec: AgentSpec,
        connect: bool,
    ) -> impl Future<Output = Status> + use<> {
        let server = self.server;
        let (send, receive) = futures::channel::oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(REQUEST_TIMEOUT, request(server, spec, connect))
                .await
                .unwrap_or_else(|_| Err("检测超时；请确认本机 basrd 和 Agent 可正常启动。".into()));
            let _ = send.send(result.unwrap_or_else(Status::Failed));
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Status::Failed("检测任务已结束，请重试。".into()))
        }
    }
}

/// # Errors
/// Rejects remote endpoints, incompatible daemons, stale configuration and transport failures.
async fn request(server: SocketAddr, spec: AgentSpec, connect: bool) -> Result<Status, String> {
    // Local settings must never launch a command on a remotely configured daemon.
    if !server.ip().is_loopback() {
        return Err("当前连接的是远端服务；本页只管理本机配置。请连接本机 basrd 后检测。".into());
    }
    let expected_spec_json = serde_json::to_vec(&spec).map_err(|error| error.to_string())?;
    let mut client = Client::connect_at(server)
        .await
        .map_err(|_| format!("无法连接本机 basrd（{server}），请先启动服务。"))?;
    for attempt in 0..RELOAD_ATTEMPTS {
        let result = client
            .check_agent(pb::CheckAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: expected_spec_json.clone(),
                connect,
            })
            .await;
        match result {
            Ok(response) => {
                let response = response.into_inner();
                return Ok(match response.status() {
                    pb::AgentCheckStatus::Connected => Status::Connected(response.resolved_program),
                    pb::AgentCheckStatus::Installed => Status::Installed(response.resolved_program),
                    pb::AgentCheckStatus::AuthenticationRequired => {
                        let initialization = response.initialization
                            .ok_or_else(|| "Agent 要求认证，但没有返回认证方式；请升级 daemon 或按工具文档登录。".to_owned())?
                            .decode::<basr::acp::InitializeResponse>()
                            .map_err(|_| "Agent 认证信息格式无效。".to_owned())?;
                        Status::AuthenticationRequired {
                            program: response.resolved_program,
                            methods: authentication_methods(
                                initialization,
                                response.authentication_error,
                            )?,
                        }
                    }
                    pb::AgentCheckStatus::MissingExecutable => Status::Missing(response.message),
                    _ => Status::Failed(response.message),
                });
            }
            Err(error)
                if matches!(
                    error.code(),
                    tonic::Code::FailedPrecondition | tonic::Code::NotFound
                ) && attempt + 1 < RELOAD_ATTEMPTS =>
            {
                // Give the daemon's configuration watcher time to apply a just-saved path.
                tokio::time::sleep(RELOAD_INTERVAL).await;
            }
            Err(error) => {
                return Err(match error.code() {
                    tonic::Code::Unimplemented => {
                        "当前 basrd 不支持连接检测，请先升级 daemon；不会假定 Agent 已连通。".into()
                    }
                    tonic::Code::FailedPrecondition | tonic::Code::NotFound => {
                        "本机 basrd 尚未加载这份配置；请检查配置路径或 --agents 覆盖，再重新检测。"
                            .into()
                    }
                    _ => format!("检测失败：{}", error.message()),
                });
            }
        }
    }
    unreachable!("the last retry returns its result")
}

/// # Errors
/// Rejects malformed native errors or explicit method lists instead of using stale methods.
fn authentication_methods(
    initialization: basr::acp::InitializeResponse,
    error: Option<pb::AcpPayload>,
) -> Result<Vec<basr::acp::AuthMethod>, String> {
    if let Some(error) = error {
        let error = error
            .decode::<basr::acp::Error>()
            .map_err(|_| "Agent 认证响应格式无效。".to_owned())?;
        if error.code != basr::acp::ErrorCode::AuthRequired {
            return Err("Agent 返回了不一致的认证状态。".into());
        }
        if let Some(methods) = error.data.as_ref().and_then(|data| data.get("authMethods")) {
            return serde_json::from_value(methods.clone())
                .map_err(|_| "Agent 返回的认证方式格式无效。".into());
        }
    }
    Ok(initialization.auth_methods)
}
