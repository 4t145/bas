//! Client-facing authentication failures retain stages without dumping RPC data.

use std::sync::LazyLock;

use bas::acp;
use regex::Regex;
use serde_json::Value;

use crate::agent::acp::AuthenticationError;

const MAX_CAUSES: usize = 8;
const MAX_TEXT_CHARS: usize = 512;
const MAX_MESSAGE_CHARS: usize = 2048;

static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+")
        .expect("the bearer credential pattern is valid")
});
static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b(?:access_token|refresh_token|id_token|api[ _-]?key|client_secret|password)\b["']?\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;]+)"#,
    )
    .expect("the explicit credential assignment pattern is valid")
});
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https?://[^\s<>"']+"#).expect("the diagnostic URL pattern is valid")
});

/// Both authentication RPCs expose the same bounded, credential-filtered causes.
pub(super) fn status(error: &anyhow::Error) -> tonic::Status {
    tonic::Status::failed_precondition(message(error))
}

fn message(error: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for cause in error.chain().take(MAX_CAUSES) {
        // SDK Error::Display serializes every data field. Read the explicitly
        // supported diagnostic text instead, including the raw-capture wrapper.
        let part = if let Some(error) = cause.downcast_ref::<AuthenticationError>() {
            protocol_error(error)
        } else if let Some(error) = cause.downcast_ref::<acp::Error>() {
            protocol_error(error)
        } else {
            diagnostic_text(&cause.to_string())
        };
        if !part.is_empty() && parts.last() != Some(&part) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return "Authentication failed".into();
    }
    bounded(&parts.join(": "), MAX_MESSAGE_CHARS)
}

fn protocol_error(error: &acp::Error) -> String {
    let message = diagnostic_text(&error.message);
    let mut result = format!("ACP error {}", i32::from(error.code));
    if !message.is_empty() {
        result.push_str(": ");
        result.push_str(&message);
    }
    // Rust Error::into_internal_error uses a string; the JS SDK 0.16.1
    // request-handler fallback uses {details: Error.message}. Agents may also use
    // {message: ...}. Only these direct string diagnostics are interoperable.
    // Other fields, nested objects, arrays and Debug representations stay private.
    let detail = error.data.as_ref().and_then(|data| {
        data.as_str()
            .or_else(|| data.get("message").and_then(Value::as_str))
            .or_else(|| data.get("details").and_then(Value::as_str))
    });
    if let Some(detail) = detail {
        let detail = diagnostic_text(detail);
        if !detail.is_empty() && detail != message {
            result.push_str(": ");
            result.push_str(&detail);
        }
    }
    result
}

fn diagnostic_text(text: &str) -> String {
    let text: String = text
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = BEARER.replace_all(&text, "Bearer [redacted]");
    let text = CREDENTIAL.replace_all(&text, "$1[redacted]");
    let text = URL.replace_all(&text, |matched: &regex::Captures<'_>| {
        let Ok(mut url) = reqwest::Url::parse(&matched[0]) else {
            return "[URL omitted]".into();
        };
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        url.to_string()
    });
    bounded(&text, MAX_TEXT_CHARS)
}

fn bounded(text: &str, limit: usize) -> String {
    let mut characters = text.chars();
    let mut result: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests;
