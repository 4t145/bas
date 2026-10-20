/// Default connection type. Resolves to same-user local IPC, never TCP.
pub const DEFAULT_TARGET: &str = "local";

/// Default port for TCP and mTLS targets that omit an explicit port.
pub const DEFAULT_PORT: u16 = 7337;

/// Conventional address for explicitly enabled, unauthenticated TCP.
pub static DEFAULT_ADDRESS: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("127.0.0.1:{DEFAULT_PORT}"));

/// Recent replayable events requested by clients unless configured otherwise.
pub const DEFAULT_REPLAY: usize = 32;

/// Maximum gRPC message size, including base64 image/file payloads and history pages.
pub const MAX_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
