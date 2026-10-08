use std::{net::SocketAddr, time::Duration};

use tonic::transport::{Channel, Endpoint};

use super::pb;

/// How long a client waits for the daemon to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A connected `basr.v1.Basr` client.
pub type Client = pb::basr_client::BasrClient<Channel>;

impl Client {
    /// Answers a live tool approval with a canonical ACP outcome.
    /// # Errors
    /// Returns an error if the request expired, the option is invalid, or transport fails.
    pub async fn respond_permission_acp(
        &mut self,
        session: crate::model::SessionId,
        request_id: String,
        outcome: crate::acp::RequestPermissionOutcome,
    ) -> Result<(), tonic::Status> {
        let response = crate::acp::RequestPermissionResponse::new(outcome);
        self.respond_permission(pb::RespondPermissionRequest {
            session: session.to_string(),
            request_id,
            response: Some((&response).into()),
        })
        .await?;
        Ok(())
    }

    /// Applies the shared limit for ACP image/file messages and history pages.
    pub fn with_payload_limit(self) -> Self {
        self.max_decoding_message_size(super::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(super::MAX_MESSAGE_SIZE)
    }

    /// Sends a complete ACP prompt; completion arrives as a TurnFinished event.
    /// # Errors
    /// Returns the daemon's rejection or a transport failure.
    pub async fn prompt_acp(
        &mut self,
        request: &crate::acp::PromptRequest,
    ) -> Result<pb::TurnAccepted, tonic::Status> {
        Ok(self
            .prompt(pb::AcpPayload::from(request))
            .await?
            .into_inner())
    }

    /// Cancels the active ACP turn.
    /// # Errors
    /// Returns the daemon's rejection or a transport failure.
    pub async fn cancel_acp(
        &mut self,
        request: &crate::acp::CancelNotification,
    ) -> Result<(), tonic::Status> {
        self.cancel(pb::AcpPayload::from(request)).await?;
        Ok(())
    }

    /// Sets an ACP configuration option and returns the complete configuration.
    /// # Errors
    /// Returns a backend, transport or response decoding failure.
    pub async fn set_config_option_acp(
        &mut self,
        request: &crate::acp::SetSessionConfigOptionRequest,
    ) -> Result<crate::acp::SetSessionConfigOptionResponse, tonic::Status> {
        self.set_config_option(pb::AcpPayload::from(request))
            .await?
            .into_inner()
            .decode()
            .map_err(|error| tonic::Status::data_loss(error.to_string()))
    }

    /// Changes the ACP session mode.
    /// # Errors
    /// Returns a backend, transport or response decoding failure.
    pub async fn set_mode_acp(
        &mut self,
        request: &crate::acp::SetSessionModeRequest,
    ) -> Result<crate::acp::SetSessionModeResponse, tonic::Status> {
        self.set_mode(pb::AcpPayload::from(request))
            .await?
            .into_inner()
            .decode()
            .map_err(|error| tonic::Status::data_loss(error.to_string()))
    }

    /// Connects to a daemon listening on `address`.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not a valid URI or the daemon does not
    /// accept the connection.
    pub async fn connect_at(address: SocketAddr) -> Result<Self, tonic::transport::Error> {
        let endpoint =
            Endpoint::from_shared(format!("http://{address}"))?.connect_timeout(CONNECT_TIMEOUT);
        Ok(Client::connect(endpoint).await?.with_payload_limit())
    }
}

/// Extracts a native ACP error returned in gRPC status details.
/// # Errors
/// Returns an error when the status does not contain a valid ACP error object.
pub fn acp_error(status: &tonic::Status) -> Result<crate::acp::Error, serde_json::Error> {
    serde_json::from_slice(status.details())
}
