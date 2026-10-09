//! The local daemon, not the GUI's PATH, determines backend availability.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use basr::{
    config::AgentSpec,
    service::{Client, pb},
};
use gpui_kit::{App, Global};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);
const RELOAD_ATTEMPTS: usize = 6;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct Service {
    server: SocketAddr,
    runtime: Handle,
}

impl Global for Service {}

pub(crate) fn init(server: SocketAddr, runtime: Handle, cx: &mut App) {
    cx.set_global(Service { server, runtime });
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Checking,
    Connecting,
    Authenticating,
    Installed(String),
    Connected(String),
    AuthenticationRequired {
        program: String,
        methods: Vec<basr::authentication::Method>,
    },
    Missing(String),
    Failed(String),
}

impl Status {
    pub(crate) fn connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Connecting | Self::Authenticating
        )
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Checking => "正在检查程序…",
            Self::Connecting => "正在检测…",
            Self::Authenticating => "正在认证…",
            Self::Installed(_) => "程序可用 · 未检测",
            Self::Connected(_) => "检测通过",
            Self::AuthenticationRequired { .. } => "需要认证",
            Self::Missing(_) => "未找到可执行文件",
            Self::Failed(_) => "检测失败",
        }
    }

    pub(crate) fn check_label(&self) -> &'static str {
        match self {
            Self::Checking | Self::Connecting => "正在检测…",
            Self::Authenticating => "重新检测",
            Self::Installed(_) => "检测",
            _ => "重新检测",
        }
    }
}

impl Service {
    pub(crate) fn address(&self) -> SocketAddr {
        self.server
    }

    pub(crate) fn runtime(&self) -> Handle {
        self.runtime.clone()
    }

    pub(crate) fn request(
        &self,
        spec: AgentSpec,
        connect: bool,
        settings_path: PathBuf,
    ) -> impl Future<Output = Status> + use<> {
        let server = self.server;
        let (send, receive) = futures::channel::oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(
                REQUEST_TIMEOUT,
                request(server, spec, connect, settings_path),
            )
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
pub(crate) async fn verified_client(
    server: SocketAddr,
    settings_path: &std::path::Path,
) -> Result<Client, String> {
    // Local settings must never launch a command on a remotely configured daemon.
    if !server.ip().is_loopback() {
        return Err("当前连接的是远端服务；本页只管理本机配置。请连接本机 basrd 后检测。".into());
    }
    let mut client = Client::connect_at(server)
        .await
        .map_err(|_| format!("无法连接本机 basrd（{server}），请先启动服务。"))?;
    let response = client
        .list_agent_catalog(pb::ListAgentCatalogRequest { refresh: false })
        .await
        .map_err(|error| format!("无法核对 daemon 配置：{}", error.message()))?
        .into_inner();
    let catalog: basr::management::Catalog = serde_json::from_slice(&response.catalog_json)
        .map_err(|_| "daemon 的目录格式无效。".to_owned())?;
    if !super::installation::settings_match(&catalog, settings_path) {
        return Err("本机 basrd 使用另一份配置文件；请核对 --settings 路径后重试。".into());
    }
    Ok(client)
}

/// # Errors
/// Rejects mismatched daemon settings, stale specs and ACP transport failures.
async fn request(
    server: SocketAddr,
    spec: AgentSpec,
    connect: bool,
    settings_path: PathBuf,
) -> Result<Status, String> {
    let mut client = verified_client(server, &settings_path).await?;
    let expected_spec_json = serde_json::to_vec(&spec).map_err(|error| error.to_string())?;
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
                return decode_response(response.into_inner());
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
                        "当前 basrd 不支持 ACP 检测，请先升级 daemon。".into()
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
/// Rejects missing or malformed ACP authentication data.
pub(crate) fn decode_response(response: pb::CheckAgentResponse) -> Result<Status, String> {
    Ok(match response.status() {
        pb::AgentCheckStatus::Connected => Status::Connected(response.resolved_program),
        pb::AgentCheckStatus::Installed => Status::Installed(response.resolved_program),
        pb::AgentCheckStatus::AuthenticationRequired => {
            let initialization = response
                .initialization
                .ok_or_else(|| {
                    "Agent 要求认证，但没有返回认证方式；请升级 daemon 或按工具文档登录。"
                        .to_owned()
                })?
                .decode::<serde_json::Value>()
                .map_err(|_| "Agent 认证信息格式无效。".to_owned())?;
            Status::AuthenticationRequired {
                program: response.resolved_program,
                methods: authentication_methods(initialization, response.authentication_error)?,
            }
        }
        pb::AgentCheckStatus::MissingExecutable => Status::Missing(response.message),
        _ => Status::Failed(response.message),
    })
}

/// # Errors
/// Rejects malformed native errors or explicit method lists instead of using stale methods.
fn authentication_methods(
    initialization: serde_json::Value,
    error: Option<pb::AcpPayload>,
) -> Result<Vec<basr::authentication::Method>, String> {
    let error = error
        .map(|payload| payload.decode::<serde_json::Value>())
        .transpose()
        .map_err(|_| "Agent 认证响应格式无效。".to_owned())?;
    basr::authentication::methods(&initialization, error.as_ref())
        .map_err(|error| format!("Agent 认证信息无效：{error}"))
}
