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
        .map_err(|_| Failure::Timeout.context(FailureContext::RpcDeadline))
        .and_then(|result| result.map_err(Failure::from));
        let failure = match result {
            Ok(response) => {
                let mut stream = response.into_inner();
                loop {
                    match stream.message().await {
                        Ok(Some(snapshot)) if snapshot.session == session.to_string() => {
                            if updates
                                .send(Update::Elicitations {
                                    epoch,
                                    pending: snapshot.pending.into_iter().map(Into::into).collect(),
                                    completed_ids: snapshot.completed_ids,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(Some(_)) => {
                            break Failure::Condition(Condition::ElicitationWrongSession);
                        }
                        Ok(None) => break Failure::Condition(Condition::ElicitationEnded),
                        Err(error) => {
                            break Failure::from(error)
                                .context(FailureContext::ElicitationDisconnected);
                        }
                    }
                }
            }
            Err(error) if error.code() == Some(Code::Unimplemented) => return,
            Err(error) if matches!(error.code(), Some(Code::NotFound | Code::InvalidArgument)) => {
                return;
            }
            Err(error) => error.context(FailureContext::ElicitationRead),
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
