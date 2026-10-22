use super::*;
use bas::{
    config::{AcpLifecycle, Daemon},
    service::pb,
};
use tonic::Status as RpcStatus;

impl Manager {
    fn policy_store(&self) -> Result<&Store, RpcStatus> {
        self.store.as_ref().ok_or_else(|| RpcStatus::failed_precondition(
            "This daemon uses a fixed agent catalog; ACP policy must be configured on the server",
        ))
    }

    /// # Errors
    /// Rejects fixed catalogs and unreadable server configuration.
    pub fn acp_configuration(&self) -> Result<pb::AcpConfiguration, RpcStatus> {
        let document = self.policy_store()?.load().map_err(configuration_error)?;
        self.acp_snapshot(&document.settings)
    }

    fn acp_snapshot(&self, settings: &Daemon) -> Result<pb::AcpConfiguration, RpcStatus> {
        let encode = |value: &Daemon| {
            serde_json::to_vec(value).map_err(|error| RpcStatus::internal(error.to_string()))
        };
        Ok(pb::AcpConfiguration {
            profile: self.profile.clone(),
            revision: format!("{:x}", Sha256::digest(encode(settings)?)),
            policy_json: serde_json::to_vec(&settings.acp)
                .map_err(|error| RpcStatus::internal(error.to_string()))?,
            registrations_json: serde_json::to_vec(&settings.agents)
                .map_err(|error| RpcStatus::internal(error.to_string()))?,
        })
    }

    /// # Errors
    /// Rejects other servers, stale snapshots, invalid policies and failed persistence.
    pub async fn save_acp_configuration(
        &self,
        request: pb::SaveAcpConfigurationRequest,
        registry: &crate::session::Registry,
    ) -> Result<pb::AcpConfiguration, RpcStatus> {
        let _guard = self.mutation.lock().await;
        let store = self.policy_store()?;
        if request.profile != self.profile {
            return Err(RpcStatus::failed_precondition(
                "Daemon changed; reload before saving",
            ));
        }
        let mut document = store.load().map_err(configuration_error)?;
        if request.revision != self.acp_snapshot(&document.settings)?.revision {
            return Err(RpcStatus::aborted(
                "ACP policy or agent catalog changed; reload before saving",
            ));
        }
        let policy: AcpLifecycle = serde_json::from_slice(&request.policy_json)
            .map_err(|error| RpcStatus::invalid_argument(error.to_string()))?;
        policy
            .validate(&document.settings.agents)
            .map_err(|error| RpcStatus::invalid_argument(error.to_string()))?;
        crate::agent::Catalog::from_registrations(document.settings.agents.clone())
            .map_err(|error| RpcStatus::invalid_argument(error.to_string()))?;
        let mut save_error = None;
        registry
            .reload_configuration(|| {
                if let Err(error) = store.save_acp(&mut document, policy) {
                    save_error = Some(error);
                    anyhow::bail!("ACP policy save failed");
                }
                Ok(document.settings.clone())
            })
            .map_err(|error| {
                save_error
                    .map(configuration_error)
                    .unwrap_or_else(|| RpcStatus::internal(error.to_string()))
            })?;
        self.acp_snapshot(&document.settings)
    }
}
