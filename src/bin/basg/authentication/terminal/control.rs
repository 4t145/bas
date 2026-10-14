//! One-use, local terminal-helper control connection. Socket ownership is the
//! cancellation boundary; no background task retains either end.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CANDIDATE_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPTED: u8 = 1;

// Deliberately no Debug: this value is a capability passed only via the
// helper's environment, never an argument, log entry, or file.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Endpoint {
    address: SocketAddr,
    token: Uuid,
}

pub(super) struct Listener {
    socket: TcpListener,
    endpoint: Endpoint,
}

pub(super) struct Controller {
    socket: TcpStream,
}

pub(super) struct Helper {
    socket: TcpStream,
}

impl Listener {
    /// Bind a local control listener inside a Tokio runtime.
    ///
    /// # Errors
    /// Returns the underlying socket setup error.
    pub(super) fn bind() -> io::Result<Self> {
        let socket = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        socket.set_nonblocking(true)?;
        let address = socket.local_addr()?;
        Ok(Self {
            socket: TcpListener::from_std(socket)?,
            endpoint: Endpoint {
                address,
                token: Uuid::new_v4(),
            },
        })
    }

    pub(super) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Wait for an authenticated helper. The caller owns the overall deadline
    /// and must drop this listener after accepting its one helper.
    ///
    /// # Errors
    /// Returns listener accept errors. Invalid, stalled, or disconnected
    /// candidates are discarded without ending the authentication attempt.
    pub(super) async fn accept(&self) -> io::Result<Controller> {
        loop {
            let (mut socket, _) = self.socket.accept().await?;
            let handshake = async {
                let mut token = [0; 16];
                socket.read_exact(&mut token).await?;
                if token != *self.endpoint.token.as_bytes() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "invalid terminal control token",
                    ));
                }
                socket.write_all(&[ACCEPTED]).await
            };
            if matches!(timeout(CANDIDATE_TIMEOUT, handshake).await, Ok(Ok(()))) {
                return Ok(Controller { socket });
            }
        }
    }
}

impl Endpoint {
    /// Connect and authenticate before the helper may start login.
    ///
    /// # Errors
    /// Rejects non-loopback addresses and invalid acknowledgements. Preserves
    /// socket errors; reports a timed-out connection or handshake as TimedOut.
    pub(super) async fn connect(&self) -> io::Result<Helper> {
        if !self.address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal control address must be loopback",
            ));
        }
        timeout(CONNECT_TIMEOUT, async {
            let mut socket = TcpStream::connect(self.address).await?;
            socket.write_all(self.token.as_bytes()).await?;
            let mut acknowledgement = [0];
            socket.read_exact(&mut acknowledgement).await?;
            if acknowledgement != [ACCEPTED] {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid terminal control acknowledgement",
                ));
            }
            Ok(Helper { socket })
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "terminal control connection timed out",
            )
        })?
    }
}

impl Controller {
    /// Receive the helper's complete signed exit code.
    ///
    /// # Errors
    /// Returns socket errors, including UnexpectedEof for missing or partial
    /// results. On cancellation the caller must drop this controller, not just
    /// the result future, to close the helper's connection.
    pub(super) async fn result(&mut self) -> io::Result<i32> {
        let mut bytes = [0; 4];
        self.socket.read_exact(&mut bytes).await?;
        Ok(i32::from_be_bytes(bytes))
    }
}

impl Helper {
    /// Wait for controller closure. Unexpected input or an I/O failure also
    /// cancels login: the controller sends no data after acknowledging us.
    pub(super) async fn cancelled(&mut self) {
        let mut byte = [0];
        let _ = self.socket.read(&mut byte).await;
    }

    /// Return the login exit code and relinquish the control connection.
    ///
    /// # Errors
    /// Returns the underlying socket write error.
    pub(super) async fn finish(mut self, code: i32) -> io::Result<()> {
        self.socket.write_all(&code.to_be_bytes()).await
    }
}

#[cfg(test)]
mod tests;
