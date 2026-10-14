//! Foreground presence belongs to the connection and is released on disconnect.

use futures::StreamExt;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

use super::*;

pub(super) async fn watch(mut client: Client, visibility: watch::Receiver<Option<SessionId>>) {
    loop {
        let requests = WatchStream::new(visibility.clone()).map(|session| pb::ForegroundRequest {
            session: session.map(|id| id.to_string()),
        });
        let result = async {
            let mut stream = client.set_foreground(requests).await?.into_inner();
            while stream.message().await?.is_some() {}
            Ok::<_, tonic::Status>(())
        }
        .await;
        if let Err(error) = result {
            if error.code() == Code::Unimplemented {
                tracing::debug!("bas does not support foreground notification suppression");
                return;
            }
            tracing::debug!(%error, "reconnecting foreground state");
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}
