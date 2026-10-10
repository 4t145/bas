//! Typed content normalization for v1 transports and v2 presentation.

use crate::{acp, acp_v2};

#[cfg(test)]
mod tests;

/// Converts a v1 content block while retaining annotations and metadata.
///
/// # Errors
/// Returns an error when the block has no compatible v2 representation.
pub fn content_from_v1(
    content: &acp::ContentBlock,
) -> Result<acp_v2::ContentBlock, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(content)?)
}

/// Converts presentation content for the existing rich-content renderer.
///
/// # Errors
/// Returns an error when the block has no compatible v1 representation.
pub fn content_to_v1(
    content: &acp_v2::ContentBlock,
) -> Result<acp::ContentBlock, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(content)?)
}
