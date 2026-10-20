//! Explicit mTLS configuration. Only file references are serializable.
//!
//! Credentials are loaded when constructing a channel or server, never from
//! mutable frontend settings during a request or a reconnect.

use std::{fmt, path::PathBuf, str::FromStr, time::Duration};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tonic::transport::{
    Certificate, Channel, ClientTlsConfig, Endpoint, Identity, ServerTlsConfig,
};

use super::Target;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientTls {
    pub ca: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerTls {
    pub client_ca: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "mTLS requires a CA, certificate and private key; TLS options require an mtls:// target"
    )]
    Configuration,
    #[error("TLS file paths and an explicit server name must not be empty")]
    Empty,
    #[error("cannot read TLS file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("TLS transport configuration or connection failed: {0}")]
    Transport(#[from] tonic::transport::Error),
}

fn read(path: &PathBuf) -> Result<Vec<u8>, Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::Empty);
    }
    std::fs::read(path).map_err(|source| Error::Read {
        path: path.clone(),
        source,
    })
}

impl ClientTls {
    /// # Errors
    /// Rejects empty references; certificate validation occurs when loading TLS.
    pub fn validate(&self) -> Result<(), Error> {
        if [&self.ca, &self.certificate, &self.private_key]
            .iter()
            .any(|path| path.as_os_str().is_empty())
            || self
                .server_name
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
        {
            return Err(Error::Empty);
        }
        Ok(())
    }

    fn load(&self) -> Result<ClientTlsConfig, Error> {
        self.validate()?;
        let mut config = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(read(&self.ca)?))
            .identity(Identity::from_pem(
                read(&self.certificate)?,
                read(&self.private_key)?,
            ));
        if let Some(name) = &self.server_name {
            config = config.domain_name(name);
        }
        Ok(config)
    }
}

impl ServerTls {
    /// # Errors
    /// Returns file read errors. Tonic validates the material when configuring
    /// the server. Client certificates are mandatory, not optional.
    pub fn load(&self) -> Result<ServerTlsConfig, Error> {
        // Tonic's server uses rustls's process default; GUI dependencies enable
        // both providers, so feature-based selection is ambiguous. Respect an
        // existing default, otherwise use the same ring provider as tonic.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(ServerTlsConfig::new()
            .identity(Identity::from_pem(
                read(&self.certificate)?,
                read(&self.private_key)?,
            ))
            .client_ca_root(Certificate::from_pem(read(&self.client_ca)?))
            .client_auth_optional(false))
    }
}

/// An owned endpoint plus credential references; include the whole value in
/// frontend cache/window keys, not just its target.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ConnectionOptions {
    pub target: Target,
    pub tls: Option<ClientTls>,
}

impl From<Target> for ConnectionOptions {
    fn from(target: Target) -> Self {
        Self { target, tls: None }
    }
}

impl fmt::Display for ConnectionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.target.fmt(formatter)
    }
}

impl FromStr for ConnectionOptions {
    type Err = super::ParseTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Target>().map(Into::into)
    }
}

impl ConnectionOptions {
    pub fn is_local(&self) -> bool {
        self.target.is_local()
    }

    pub fn is_loopback_tcp(&self) -> bool {
        self.target.is_loopback_tcp()
    }

    /// # Errors
    /// Returns local endpoint resolution errors. TLS identities have separate
    /// frontend state, even when they connect to the same server.
    pub fn state_key(&self) -> std::io::Result<String> {
        let key = self.target.state_key()?;
        Ok(match &self.tls {
            Some(tls) => format!(
                "{key}-{:x}",
                Sha256::digest(serde_json::to_vec(tls).map_err(std::io::Error::other)?)
            ),
            None => key,
        })
    }

    /// # Errors
    /// Rejects missing mTLS credentials and TLS options on non-TLS targets.
    pub fn validate(&self) -> Result<(), Error> {
        match (&self.target, &self.tls) {
            (Target::Mtls(_), Some(tls)) => tls.validate(),
            (Target::Mtls(_), None) | (_, Some(_)) => Err(Error::Configuration),
            _ => Ok(()),
        }
    }

    fn tls_endpoint(&self, timeout: Duration) -> Result<Option<Endpoint>, Error> {
        self.validate()?;
        match (&self.target, &self.tls) {
            (Target::Mtls(address), Some(tls)) => Ok(Some(
                Endpoint::from_shared(format!("https://{address}"))?
                    .connect_timeout(timeout)
                    .tls_config(tls.load()?)?,
            )),
            _ => Ok(None),
        }
    }

    /// Loads and pins credentials before constructing a reconnecting channel.
    /// # Errors
    /// Returns invalid configuration, unreadable files, or invalid TLS material.
    pub fn channel_lazy(&self, timeout: Duration) -> Result<Channel, Error> {
        Ok(match self.tls_endpoint(timeout)? {
            Some(endpoint) => endpoint.connect_lazy(),
            None => self.target.channel_lazy(timeout),
        })
    }

    /// # Errors
    /// Also returns TLS authentication, name verification, and connection errors.
    pub async fn channel(&self, timeout: Duration) -> Result<Channel, Error> {
        Ok(match self.tls_endpoint(timeout)? {
            Some(endpoint) => endpoint.connect().await?,
            None => self.target.channel(timeout).await?,
        })
    }
}
