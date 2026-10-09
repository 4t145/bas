//! Owns the daemon authentication stream; dropping it cancels its ACP process.
use std::path::PathBuf;

use basr::{authentication::Method, config::AgentSpec, service::pb};
use futures::channel::mpsc as futures_mpsc;
use tokio::{runtime::Handle, sync::mpsc};
use tokio_util::task::AbortOnDropHandle;

use crate::settings::agents::check;

pub(super) struct Stream {
    pub input: mpsc::UnboundedSender<pb::AuthenticationInput>,
    pub events: futures_mpsc::UnboundedReceiver<Result<pb::AuthenticationEvent, String>>,
    pub task: AbortOnDropHandle<()>,
}

pub(super) fn start(
    service: check::Service,
    spec: AgentSpec,
    method: Method,
    path: PathBuf,
) -> Stream {
    let (input, receive) = mpsc::unbounded_channel();
    let (events, receiver) = futures_mpsc::unbounded();
    let runtime = service.runtime();
    let task = AbortOnDropHandle::new(runtime.spawn(async move {
        let result = run(service, spec, method, path, receive, events.clone()).await;
        if let Err(error) = result {
            let _ = events.unbounded_send(Err(error));
        }
    }));
    Stream {
        input,
        events: receiver,
        task,
    }
}

async fn run(
    service: check::Service,
    spec: AgentSpec,
    method: Method,
    path: PathBuf,
    mut input: mpsc::UnboundedReceiver<pb::AuthenticationInput>,
    events: futures_mpsc::UnboundedSender<Result<pb::AuthenticationEvent, String>>,
) -> Result<(), String> {
    let mut client = check::verified_client(service.address(), &path).await?;
    let saved = basr::config::Store::at(path)
        .load()
        .map_err(|_| "无法核对 Agent 配置；请重新检测。".to_owned())?;
    if !saved.settings.daemon.agents.contains(&spec) {
        return Err("Agent 配置已变化；请重新检测后再认证。".into());
    }
    let request = pb::AuthenticationInput {
        kind: Some(pb::authentication_input::Kind::Start(
            pb::AuthenticateAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: serde_json::to_vec(&spec).map_err(|error| error.to_string())?,
                method_id: method.id.clone(),
                expected_method_json: serde_json::to_vec(&method)
                    .map_err(|error| error.to_string())?,
            },
        )),
    };
    let (send, receive) = mpsc::channel(16);
    send.send(request)
        .await
        .map_err(|_| "认证请求已结束。".to_owned())?;
    let mut stream = client
        .authenticate_agent_interactive(tokio_stream::wrappers::ReceiverStream::new(receive))
        .await
        .map_err(|error| rpc_error("无法开始认证", &error))?
        .into_inner();
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()); };
                send.send(next).await.map_err(|_| "认证连接已关闭。".to_owned())?;
            }
            next = stream.message() => {
                let event = next.map_err(|error| rpc_error("认证失败", &error))?
                    .ok_or_else(|| "认证未完成，连接已关闭；可重新检测后重试。".to_owned())?;
                let complete = matches!(event.kind, Some(pb::authentication_event::Kind::Check(_)));
                if events.unbounded_send(Ok(event)).is_err() { return Ok(()); }
                if complete { return Ok(()); }
            }
        }
    }
}

fn rpc_error(action: &str, error: &tonic::Status) -> String {
    if error.code() == tonic::Code::Unimplemented {
        return "当前运行的 basrd 不支持交互认证。更新后请重启 basrd 服务，再重新检测。".into();
    }
    let message = error.message().trim();
    if message.is_empty() {
        format!(
            "{action}（{:?}）：服务未返回详细原因，请重新检测后重试。",
            error.code()
        )
    } else {
        format!("{action}：{message}")
    }
}

pub(super) fn terminal(
    runtime: &Handle,
    plan: basr::authentication::Invocation,
) -> impl Future<Output = Result<i32, String>> + use<> {
    let (send, receive) = futures::channel::oneshot::channel();
    runtime.spawn(async move {
        let result = async {
            let invocation =
                super::terminal::Invocation::from_plan(plan).map_err(|error| error.to_string())?;
            let login = super::terminal::launch(invocation).map_err(|error| error.to_string())?;
            login.complete_code().await
        }
        .await;
        let _ = send.send(result);
    });
    async move {
        receive
            .await
            .unwrap_or_else(|_| Err("认证终端任务已结束。".into()))
    }
}
