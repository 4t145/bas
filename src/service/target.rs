use std::{fmt, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use sha2::{Digest, Sha256};
use tonic::transport::{Channel, Endpoint};

use super::DEFAULT_PORT;

#[cfg(test)]
mod tests;

/// A transport endpoint, not an account or an authentication credential.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Target {
    /// The current user's platform-specific local IPC endpoint.
    #[default]
    Local,
    /// An absolute Unix-domain socket path.
    Unix(PathBuf),
    /// A local Windows named-pipe name, without a UNC prefix.
    Pipe(String),
    /// Explicitly selected plaintext TCP. Loopback is not user authentication.
    Tcp(TcpAddress),
    /// Mutually authenticated TLS; requires explicit connection credentials.
    Mtls(TcpAddress),
}

/// A numeric IP or system-resolved hostname and numeric port.
///
/// Parsing never performs DNS resolution. Numeric addresses are canonicalized;
/// hostname spelling is retained so configuration and state do not depend on DNS.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TcpAddress(String);

impl TcpAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// UI authority, omitting only the default port. Transport and configuration
    /// should use `as_str` or `Display` to retain the explicit canonical port.
    pub fn display_compact(&self) -> String {
        let suffix = format!(":{DEFAULT_PORT}");
        self.0.strip_suffix(&suffix).unwrap_or(&self.0).to_owned()
    }
}

impl From<SocketAddr> for TcpAddress {
    fn from(address: SocketAddr) -> Self {
        Self(address.to_string())
    }
}

impl fmt::Display for TcpAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for TcpAddress {
    type Err = ParseTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Ok(address) = value.parse::<SocketAddr>() {
            return Ok(address.into());
        }
        let error =
            ParseTargetError("TCP requires an IP or hostname with an optional numeric port");
        if let Some(ip) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let ip: std::net::Ipv6Addr = ip.parse().map_err(|_| error.clone())?;
            return Ok(SocketAddr::new(ip.into(), DEFAULT_PORT).into());
        }
        let (host, port) = match value.rsplit_once(':') {
            Some((host, port)) => {
                if port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) {
                    return Err(error);
                }
                (host, port.parse::<u16>().map_err(|_| error.clone())?)
            }
            None => (value, DEFAULT_PORT),
        };
        // Accept system hostname labels, including hosts-file underscores and
        // an optional trailing DNS root dot, but never URI authority extras.
        if host.is_empty()
            || !host.trim_end_matches('.').split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            })
            || host.ends_with("..")
        {
            return Err(error);
        }
        let authority = format!("{host}:{port}");
        // The client constructs an HTTP/2 endpoint from this authority. Reject
        // inputs outside the URI parser's limits here, rather than panicking later.
        format!("http://{authority}")
            .parse::<tonic::codegen::http::Uri>()
            .map_err(|_| error)?;
        Ok(Self(authority))
    }
}

/// Invalid connection-target syntax.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid connection target: {0}")]
pub struct ParseTargetError(&'static str);

impl FromStr for Target {
    type Err = ParseTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "local" {
            return Ok(Self::Local);
        }
        if let Some(path) = value.strip_prefix("unix://") {
            // Unix syntax is parsed consistently even on a Windows client.
            if !path.starts_with('/') || path == "/" || path.contains('\0') {
                return Err(ParseTargetError("unix:// requires an absolute socket path"));
            }
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        if let Some(name) = value.strip_prefix("pipe://") {
            if name.is_empty()
                || name == "."
                || name == ".."
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err(ParseTargetError(
                    "pipe:// requires a local name containing only letters, digits, '.', '_' or '-'",
                ));
            }
            return Ok(Self::Pipe(name.to_owned()));
        }
        if let Some(address) = value.strip_prefix("mtls://") {
            return address.parse().map(Self::Mtls);
        }
        value
            .strip_prefix("tcp://")
            .unwrap_or(value)
            .parse()
            .map(Self::Tcp)
            .map_err(|_| {
                ParseTargetError(
                    "expected local, unix:///path, pipe://name, mtls://HOST[:PORT], tcp://HOST[:PORT] or HOST[:PORT]",
                )
            })
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => formatter.write_str("local"),
            Self::Unix(path) => write!(formatter, "unix://{}", path.display()),
            Self::Pipe(name) => write!(formatter, "pipe://{name}"),
            // Preserve existing command output and TCP state filenames.
            Self::Tcp(address) => address.fmt(formatter),
            Self::Mtls(address) => write!(formatter, "mtls://{address}"),
        }
    }
}

impl From<SocketAddr> for Target {
    fn from(address: SocketAddr) -> Self {
        Self::Tcp(address.into())
    }
}

impl Target {
    /// Whether this target requests OS-authenticated local IPC.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local | Self::Unix(_) | Self::Pipe(_))
    }

    /// Legacy TCP locality, for existing UI restrictions, not authentication.
    pub fn is_loopback_tcp(&self) -> bool {
        matches!(self, Self::Tcp(address)
            if address.as_str().parse::<SocketAddr>().is_ok_and(|address| address.ip().is_loopback()))
    }

    /// Resolves the local alias to the current user's platform endpoint.
    ///
    /// # Errors
    /// Returns an error if the platform or the current user's local endpoint
    /// cannot be determined.
    pub fn resolved(&self) -> std::io::Result<Self> {
        super::transport::resolve(self)
    }

    /// A filename-safe endpoint state key, preserving existing TCP state.
    ///
    /// # Errors
    /// Returns an error if the local endpoint cannot be determined. The local
    /// alias and its explicit socket or pipe target share the same state.
    pub fn state_key(&self) -> std::io::Result<String> {
        let target = self.resolved()?;
        Ok(match &target {
            Self::Tcp(address) if address.as_str().parse::<SocketAddr>().is_ok() => {
                address.to_string().replace([':', '[', ']'], "_")
            }
            Self::Tcp(address) => {
                format!("tcp-host-{:x}", Sha256::digest(address.as_str().as_bytes()))
            }
            Self::Mtls(address) => {
                format!("mtls-{:x}", Sha256::digest(address.as_str().as_bytes()))
            }
            _ => format!("ipc-{:x}", Sha256::digest(target.to_string().as_bytes())),
        })
    }

    /// Constructs a reconnecting channel without connecting immediately.
    ///
    /// Local IPC identity checks run on every connection, including reconnects.
    /// Unsupported local transports fail on use; they never fall back to TCP.
    /// mTLS requires [`super::tls::ConnectionOptions`]; without credentials this
    /// legacy API fails on connection and never sends plaintext to an mTLS target.
    pub fn channel_lazy(&self, timeout: Duration) -> Channel {
        let endpoint = self.endpoint(timeout);
        match self {
            Self::Tcp(_) => endpoint.connect_lazy(),
            _ => endpoint.connect_with_connector_lazy(super::transport::connector(self.clone())),
        }
    }

    pub(super) async fn channel(
        &self,
        timeout: Duration,
    ) -> Result<Channel, tonic::transport::Error> {
        let endpoint = self.endpoint(timeout);
        match self {
            Self::Tcp(_) => endpoint.connect().await,
            _ => {
                endpoint
                    .connect_with_connector(super::transport::connector(self.clone()))
                    .await
            }
        }
    }

    fn endpoint(&self, timeout: Duration) -> Endpoint {
        match self {
            Self::Tcp(address) => Endpoint::from_shared(format!("http://{address}"))
                .expect("a validated TCP address produces a valid HTTP URI"),
            // HTTP/2 authority only; the connector never resolves this hostname.
            _ => Endpoint::from_static("http://bas.local"),
        }
        .connect_timeout(timeout)
    }
}
