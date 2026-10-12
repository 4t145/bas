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
        .unwrap_or_else(|_| {
            Err(tonic::Status::deadline_exceeded(crate::i18n::tr!(
                "connection-request-timeout"
            )))
        });
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
                        Ok(Some(_)) => break crate::i18n::tr!("interaction-wrong-session").into(),
                        Ok(None) => break crate::i18n::tr!("interaction-ended").into(),
                        Err(error) => {
                            break crate::i18n::tr!(
                                "interaction-disconnected",
                                error = (error).to_string()
                            );
                        }
                    }
                }
            }
            Err(error) if error.code() == Code::Unimplemented => return,
            Err(error) if matches!(error.code(), Code::NotFound | Code::InvalidArgument) => return,
            Err(error) => crate::i18n::tr!("interaction-read-failed", error = (error).to_string()),
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
