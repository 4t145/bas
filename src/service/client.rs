use std::{net::SocketAddr, time::Duration};

use tonic::transport::{Channel, Endpoint};

use super::pb;

/// How long a client waits for the daemon to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A connected `basr.v1.Basr` client.
pub type Client = pb::basr_client::BasrClient<Channel>;

/// Connects to a daemon listening on `address`.
///
/// # Errors
///
/// Returns an error when the address is not a valid URI or the daemon does not
/// accept the connection.
pub async fn connect(address: SocketAddr) -> Result<Client, tonic::transport::Error> {
    let endpoint =
        Endpoint::from_shared(format!("http://{address}"))?.connect_timeout(CONNECT_TIMEOUT);
    Client::connect(endpoint).await
}
