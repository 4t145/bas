use super::*;

pub(super) async fn watch(
    mut client: Client,
    updates: mpsc::Sender<Update>,
    session: SessionId,
    epoch: u64,
) {
    loop {
        let result = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client.watch_elicitations(pb::WatchElicitationsRequest {
                session: session.to_string(),
            }),
        )
        .await
        .unwrap_or_else(|_| Err(tonic::Status::deadline_exceeded(REQUEST_TIMEOUT_MESSAGE)));
        let failure = match result {
            Ok(response) => {
                let mut stream = response.into_inner();
                loop {
                    match stream.message().await {
                        Ok(Some(snapshot)) if snapshot.session == session.to_string() => {
                            if updates
                                .send(Update::Elicitations {
                                    epoch,
                                    pending: snapshot.pending,
                                    completed_ids: snapshot.completed_ids,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(Some(_)) => break "Agent 交互数据属于其他会话，已断开连接。".into(),
                        Ok(None) => break "Agent 交互连接已结束。".into(),
                        Err(error) => break format!("Agent 交互连接中断：{error}"),
                    }
                }
            }
            Err(error) if error.code() == Code::Unimplemented => return,
            Err(error) if matches!(error.code(), Code::NotFound | Code::InvalidArgument) => return,
            Err(error) => format!("无法读取 Agent 交互请求：{error}"),
        };
        if updates
            .send(Update::ElicitationUnavailable {
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
