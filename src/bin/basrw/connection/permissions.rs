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
    .unwrap_or_else(|_| {
        Err(tonic::Status::deadline_exceeded(crate::i18n::tr!(
            "connection-request-timeout"
        )))
    })
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
                    Ok(None) => break crate::i18n::tr!("approval-closed").to_owned(),
                    Err(error) => {
                        break crate::i18n::tr!(
                            "approval-disconnected",
                            error = (error).to_string()
                        );
                    }
                };
                if snapshot.session != session.to_string() {
                    break crate::i18n::tr!("approval-wrong-session").to_owned();
                }
                let pending = snapshot
                    .pending
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<PendingPermission>, _>>();
                let pending = match pending {
                    Ok(pending) => pending,
                    Err(error) => {
                        break crate::i18n::tr!("approval-invalid", error = (error).to_string());
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
                    Code::Unimplemented | Code::NotFound | Code::InvalidArgument
                ) =>
            {
                let message = if status.code() == Code::Unimplemented {
                    crate::i18n::tr!("approval-upgrade").into()
                } else {
                    crate::i18n::tr!("approval-subscribe-failed", status = (status).to_string())
                };
                let _ = updates
                    .send(Update::PermissionUnavailable { epoch, message })
                    .await;
                return;
            }
            Err(error) => crate::i18n::tr!("approval-connect-failed", error = (error).to_string()),
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
