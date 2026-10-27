//! Platform-authenticated local IPC and explicitly enabled plaintext TCP.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use hyper_util::rt::TokioIo;
#[cfg(feature = "server")]
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
#[cfg(feature = "server")]
use tokio::net::{TcpListener, TcpStream};
#[cfg(feature = "server")]
use tonic::transport::server::Connected;

use super::Target;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// Identity established by the transport, not by RPC metadata.
#[cfg(feature = "server")]
#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    same_user: bool,
    tcp_peer: Option<SocketAddr>,
}

#[cfg(feature = "server")]
impl ConnectionInfo {
    /// True only after the local IPC implementation verified the OS user.
    pub fn is_same_user(&self) -> bool {
        self.same_user
    }

    /// TCP peer address, if TCP was explicitly selected. Not an authenticated user.
    pub fn tcp_peer(&self) -> Option<SocketAddr> {
        self.tcp_peer
    }
}

/// A bound listener. Owns any local endpoint's lock and cleanup lifecycle.
#[cfg(feature = "server")]
pub struct Listener {
    target: Target,
    inner: ListenerKind,
}

#[cfg(feature = "server")]
enum ListenerKind {
    Tcp(TcpListener),
    #[cfg(any(unix, windows))]
    Local(platform::Listener),
}

#[cfg(feature = "server")]
impl Listener {
    /// Binds one explicitly selected transport; never opens a fallback listener.
    ///
    /// # Errors
    /// Returns binding, local endpoint security, or unsupported-platform errors.
    pub async fn bind(target: &Target) -> io::Result<Self> {
        if matches!(target, Target::Mtls(_)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mTLS listeners require tonic per-listener TLS configuration",
            ));
        }
        if let Target::Tcp(address) = target {
            let listener = TcpListener::bind(address.as_str()).await?;
            return Ok(Self {
                target: Target::from(listener.local_addr()?),
                inner: ListenerKind::Tcp(listener),
            });
        }
        #[cfg(any(unix, windows))]
        {
            let target = resolve(target)?;
            let listener = platform::Listener::bind(&target).await?;
            Ok(Self {
                target: listener.target().clone(),
                inner: ListenerKind::Local(listener),
            })
        }
        #[cfg(not(any(unix, windows)))]
        Err(unsupported())
    }

    /// The actual bound endpoint, including an assigned ephemeral TCP port.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Accepts connections while retaining local endpoint ownership.
    pub fn incoming(self) -> impl futures::Stream<Item = io::Result<Connection>> + Send {
        futures::stream::unfold(self, |mut listener| async move {
            let connection = listener.accept().await;
            Some((connection, listener))
        })
    }

    async fn accept(&mut self) -> io::Result<Connection> {
        match &mut self.inner {
            ListenerKind::Tcp(listener) => {
                let (stream, address) = listener.accept().await?;
                Ok(Connection {
                    io: Stream::Tcp(stream),
                    #[cfg(feature = "server")]
                    info: ConnectionInfo {
                        same_user: false,
                        tcp_peer: Some(address),
                    },
                })
            }
            #[cfg(any(unix, windows))]
            ListenerKind::Local(listener) => Ok(Connection {
                io: Stream::Local(listener.accept().await?),
                info: ConnectionInfo {
                    same_user: true,
                    tcp_peer: None,
                },
            }),
        }
    }
}

pub(super) fn resolve(target: &Target) -> io::Result<Target> {
    match target {
        #[cfg(any(unix, windows))]
        Target::Local => platform::default_target(),
        #[cfg(not(any(unix, windows)))]
        Target::Local => Err(unsupported()),
        _ => Ok(target.clone()),
    }
}

#[cfg(not(any(unix, windows)))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "local IPC is unsupported on this platform",
    )
}

pub(super) fn connector(
    target: Target,
) -> impl tower::Service<
    tonic::codegen::http::Uri,
    Response = TokioIo<Connection>,
    Error = io::Error,
    Future = impl Future<Output = io::Result<TokioIo<Connection>>> + Send,
> + Clone {
    tower::service_fn(move |_| {
        let target = target.clone();
        async move {
            #[cfg(any(unix, windows))]
            {
                let target = resolve(&target)?;
                let stream = platform::connect(&target).await?;
                Ok(TokioIo::new(Connection {
                    io: Stream::Local(stream),
                    #[cfg(feature = "server")]
                    info: ConnectionInfo {
                        same_user: true,
                        tcp_peer: None,
                    },
                }))
            }
            #[cfg(not(any(unix, windows)))]
            {
                let _ = target;
                Err(unsupported())
            }
        }
    })
}

/// I/O whose connection information cannot be supplied by an RPC caller.
pub struct Connection {
    io: Stream,
    #[cfg(feature = "server")]
    info: ConnectionInfo,
}

enum Stream {
    #[cfg(feature = "server")]
    Tcp(TcpStream),
    #[cfg(any(unix, windows))]
    Local(platform::Stream),
}

#[cfg(feature = "server")]
impl Connected for Connection {
    type ConnectInfo = ConnectionInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.info.clone()
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.get_mut().io {
            #[cfg(feature = "server")]
            Stream::Tcp(stream) => Pin::new(stream).poll_read(cx, buffer),
            #[cfg(any(unix, windows))]
            Stream::Local(stream) => Pin::new(stream).poll_read(cx, buffer),
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().io {
            #[cfg(feature = "server")]
            Stream::Tcp(stream) => Pin::new(stream).poll_write(cx, buffer),
            #[cfg(any(unix, windows))]
            Stream::Local(stream) => Pin::new(stream).poll_write(cx, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().io {
            #[cfg(feature = "server")]
            Stream::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(any(unix, windows))]
            Stream::Local(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().io {
            #[cfg(feature = "server")]
            Stream::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(any(unix, windows))]
            Stream::Local(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}
