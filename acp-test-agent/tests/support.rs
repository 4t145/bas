//! Shared harness for the stdio integration tests.
//!
//! Cargo also compiles this file as an empty test target; the helpers are used
//! by `v1.rs` and `v2.rs`.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol::{AcpAgent, AcpAgentConfig};

/// Build a launcher for the test agent binary with custom arguments.
pub fn agent_with_args(args: impl IntoIterator<Item = String>) -> AcpAgent {
    AcpAgent::new(AcpAgentConfig::new(env!("CARGO_BIN_EXE_acp-test-agent")).args(args))
}

/// Build a launcher with a specific session-list page size.
pub fn agent(page_size: usize) -> AcpAgent {
    agent_with_args(["--page-size".to_string(), page_size.to_string()])
}

/// Absolute working directory used by sessions in tests.
pub fn cwd() -> PathBuf {
    std::env::current_dir().expect("test process has a working directory")
}

/// Timeout used for updates that should arrive promptly.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(5);
