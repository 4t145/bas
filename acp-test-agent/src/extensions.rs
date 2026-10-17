//! Custom `_demo/*` extension messages used by the `/extension` command.
//!
//! These are ordinary ACP extension types: the method names start with `_` and
//! the payloads are implementation-specific. Keeping them in the library lets
//! integration tests and demo clients reuse the exact same definitions.

use agent_client_protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use serde::{Deserialize, Serialize};

/// Extension request sent from the agent to the client.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcRequest)]
#[request(method = "_demo/echo", response = EchoResponse)]
#[serde(rename_all = "camelCase")]
pub struct EchoRequest {
    /// Text to echo back.
    pub text: String,
}

/// Response to [`EchoRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcResponse)]
#[serde(rename_all = "camelCase")]
pub struct EchoResponse {
    /// Text supplied by the client.
    pub text: String,
}

/// Extension notification sent from the agent to the client.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcNotification)]
#[notification(method = "_demo/notice")]
#[serde(rename_all = "camelCase")]
pub struct DemoNotice {
    /// Informational message for the client.
    pub message: String,
}
