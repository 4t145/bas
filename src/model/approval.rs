use crate::acp;
use serde::{Deserialize, Serialize};

/// A live tool approval. IDs belong to the daemon and are never reused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingPermission {
    pub request_id: String,
    pub request: acp::RequestPermissionRequest,
}
