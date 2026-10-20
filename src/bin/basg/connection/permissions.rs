use super::*;

/// Open the approval stream before loading history so a live UI owns permissions.
pub(super) async fn watch_session(
    mut client: Client,
    updates: mpsc::Sender<Update>,
    session: SessionId,
    epoch: u64,
) {
    tracing::debug!(%session, epoch, "Opening conversation permissions");
    let stream = open(&mut client, session).await;
    if let Err(error) = &stream {
        tracing::warn!(%session, epoch, %error, "Conversation permissions unavailable");
    }
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
) -> Result<tonic::Streaming<pb::PermissionSnapshot>, Failure> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        client.watch_permissions(pb::WatchPermissionsRequest {
            session: session.to_string(),
        }),
    )
    .await
    .map_err(|_| Failure::Timeout.context(FailureContext::RpcDeadline))
    .and_then(|result| result.map_err(Failure::from))
    .map(tonic::Response::into_inner)
}

async fn watch_permissions(
    mut client: Client,
    updates: mpsc::Sender<Update>,
    session: SessionId,
    epoch: u64,
    initial: Result<tonic::Streaming<pb::PermissionSnapshot>, Failure>,
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
                    Ok(None) => break Failure::Condition(Condition::PermissionStreamClosed),
                    Err(error) => {
                        break Failure::from(error).context(FailureContext::PermissionDisconnected);
                    }
                };
                if snapshot.session != session.to_string() {
                    break Failure::Condition(Condition::PermissionWrongSession);
                }
                let pending = snapshot
                    .pending
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<PendingPermission>, _>>();
                let pending = match pending {
                    Ok(pending) => pending,
                    Err(error) => {
                        break Failure::InvalidPayload(error.to_string())
                            .context(FailureContext::InvalidPermission);
                    }
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
                    Some(Code::Unimplemented | Code::NotFound | Code::InvalidArgument)
                ) =>
            {
                let message = if status.code() == Some(Code::Unimplemented) {
                    status.context(FailureContext::PermissionsUnsupported)
                } else {
                    status.context(FailureContext::PermissionSubscription)
                };
                let _ = updates
                    .send(Update::PermissionUnavailable { epoch, message })
                    .await;
                return;
            }
            Err(error) => error.context(FailureContext::PermissionConnection),
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
