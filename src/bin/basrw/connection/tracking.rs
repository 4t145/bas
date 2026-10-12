use super::*;

/// Reads only the accepted turn's authoritative outcome, independently of the
/// selected conversation. Reconnects resume after the last decoded event.
///
/// # Errors
/// Returns an error if history is unavailable, malformed, or ends without the
/// accepted turn's outcome.
pub(super) async fn track(
    mut client: Client,
    session: SessionId,
    receipt: TurnReceipt,
) -> Result<Event, String> {
    let mut cursor = receipt.cursor.raw();
    loop {
        let response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client.attach(pb::AttachRequest {
                session: session.to_string(),
                cursor: Some(cursor),
                replay: 0,
            }),
        )
        .await;
        let mut stream = match response {
            Ok(Ok(response)) => response.into_inner(),
            Ok(Err(error)) if !retryable(&error) => return Err(error.to_string()),
            _ => {
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };
        loop {
            match stream.message().await {
                Ok(Some(event)) => {
                    let event = Event::try_from(event).map_err(|error| {
                        crate::i18n::tr!("session-invalid-event", error = (error).to_string())
                    })?;
                    if event.session != session {
                        return Err(crate::i18n::tr!("turn-tracking-wrong-session").into());
                    }
                    if event.is_transient() || event.cursor.raw() <= cursor {
                        continue;
                    }
                    cursor = event.cursor.raw();
                    if matches!(
                        event.kind,
                        EventKind::TurnFinished { turn, .. }
                            | EventKind::TurnInterrupted { turn, .. }
                            if turn == receipt.turn
                    ) {
                        return Ok(event);
                    }
                }
                Ok(None) => return Err(crate::i18n::tr!("turn-tracking-ended").into()),
                Err(error) if !retryable(&error) => return Err(error.to_string()),
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    break;
                }
            }
        }
    }
}

fn retryable(error: &tonic::Status) -> bool {
    matches!(error.code(), Code::Unavailable | Code::DeadlineExceeded)
}
