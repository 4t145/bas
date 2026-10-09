use super::*;

/// Open the approval stream before loading history so a live UI owns permissions.
pub(super) async fn watch_session(
    mut client: Client,
    updates: mpsc::Sender<Update>,
    session: SessionId,
    epoch: u64,
) {
    let stream = open(&mut client, session).await;
    let approvals = watch_permissions(client.clone(), updates.clone(), session, epoch, stream);
    let elicitations = super::elicitations::watch(client.clone(), updates.clone(), session, epoch);
    tokio::join!(
        super::watch(client, updates, session, epoch),
        approvals,
        elicitations
    );
}

async fn open(
    client: &mut Client,
    session: SessionId,
) -> Result<tonic::Streaming<pb::PermissionSnapshot>, tonic::Status> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        client.watch_permissions(pb::WatchPermissionsRequest {
            session: session.to_string(),
        }),
    )
    .await
    .unwrap_or_else(|_| Err(tonic::Status::deadline_exceeded(REQUEST_TIMEOUT_MESSAGE)))
    .map(tonic::Response::into_inner)
}

async fn watch_permissions(
    mut client: Client,
    updates: mpsc::Sender<Update>,
    session: SessionId,
    epoch: u64,
    initial: Result<tonic::Streaming<pb::PermissionSnapshot>, tonic::Status>,
) {
    let mut next = Some(initial);
    loop {
        let stream = match next.take() {
            Some(initial) => initial,
            None => open(&mut client, session).await,
        };
        let failure = match stream {
            Ok(mut stream) => loop {
                let snapshot = match stream.message().await {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => break "审批连接已关闭，待审批请求已取消。".to_owned(),
                    Err(error) => break format!("审批连接中断，待审批请求已取消：{error}"),
                };
                if snapshot.session != session.to_string() {
                    break "审批数据属于其他会话，已断开审批连接。".to_owned();
                }
                let pending = snapshot
                    .pending
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<PendingPermission>, _>>();
                let pending = match pending {
                    Ok(pending) => pending,
                    Err(error) => break format!("无效的工具审批数据：{error}"),
                };
                if updates
                    .send(Update::Permissions { epoch, pending })
                    .await
                    .is_err()
                {
                    return;
                }
            },
            Err(status)
                if matches!(
                    status.code(),
                    Code::Unimplemented | Code::NotFound | Code::InvalidArgument
                ) =>
            {
                let message = if status.code() == Code::Unimplemented {
                    "当前 basrd 不支持交互审批，请升级并重启服务。".into()
                } else {
                    format!("无法订阅工具审批：{status}")
                };
                let _ = updates
                    .send(Update::PermissionUnavailable { epoch, message })
                    .await;
                return;
            }
            Err(error) => format!("无法连接工具审批服务：{error}"),
        };
        if updates
            .send(Update::PermissionUnavailable {
                epoch,
                message: failure,
            })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}
