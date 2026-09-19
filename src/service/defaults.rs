/// Default address used by the daemon and its clients.
///
/// The service listens on loopback because sessions can access local files.
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:7337";

/// Recent replayable events requested by clients unless configured otherwise.
pub const DEFAULT_REPLAY: usize = 32;

/// Maximum gRPC message size, including base64 image/file payloads and history pages.
pub const MAX_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
