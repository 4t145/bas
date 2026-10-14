use anyhow::Context as _;
use bas::acp;
use serde_json::json;

use super::{MAX_MESSAGE_CHARS, diagnostic_text, message, status};

#[test]
fn rpc_errors_keep_stage_code_and_only_supported_diagnostic_text() {
    let error = Err::<(), _>(acp::Error::internal_error().data(json!({
        "message": "Provider configuration is incomplete",
        "access_token": "private access token",
        "env": {"API_KEY": "private environment"},
        "stderr": "private stderr",
        "nested": {"message": "private nested message"}
    })))
    .context("Agent authentication failed")
    .expect_err("fixture error");
    assert_eq!(
        status(&error).message(),
        "Agent authentication failed: ACP error -32603: Internal error: Provider configuration is incomplete"
    );
    for omitted in ["private", "access_token", "env", "stderr", "nested"] {
        assert!(!message(&error).contains(omitted));
    }
}

#[test]
fn sdk_string_diagnostics_and_non_rpc_causes_are_retained() {
    let error = Err::<(), _>(
        acp::Error::new(-32602, "Provider rejected authentication").data("Project is missing"),
    )
    .context("Session creation failed after authentication")
    .expect_err("fixture error");
    assert_eq!(
        message(&error),
        "Session creation failed after authentication: ACP error -32602: Provider rejected authentication: Project is missing"
    );
    let error = Err::<(), _>(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Permission denied",
    ))
    .context("Could not start Agent")
    .expect_err("fixture error");
    assert_eq!(message(&error), "Could not start Agent: Permission denied");
    let error = anyhow::Error::new(acp::Error::internal_error().data(json!({
        "details": "Could not save authentication settings",
        "stack": "private stack",
        "credentials": {"access_token":"private credential"}
    })));
    assert_eq!(
        message(&error),
        "ACP error -32603: Internal error: Could not save authentication settings"
    );
}

#[test]
fn structured_or_duplicate_details_are_not_dumped() {
    for data in [
        json!([{"message":"private array"}]),
        json!({"message":{"cause":"private object"}}),
        json!({"message":false,"secret":"private field"}),
        json!({"details":{"message":"private nested details"}}),
        json!({"message":"Internal error"}),
    ] {
        let error = anyhow::Error::new(acp::Error::internal_error().data(data));
        assert_eq!(message(&error), "ACP error -32603: Internal error");
    }
}

#[test]
fn explicit_credentials_and_oauth_callback_secrets_are_filtered() {
    let text = "Provider rejected authentication: access_token=access-secret refresh_token: 'refresh-secret' api_key=key-secret client_secret=client-secret password=pass-secret Authorization: Bearer bearer-secret at https://user:password@accounts.example.test/callback?code=oauth-secret&state=state-secret#fragment-secret";
    let filtered = diagnostic_text(text);
    assert!(filtered.contains("Provider rejected authentication"));
    assert!(filtered.contains("https://accounts.example.test/callback"));
    for secret in [
        "access-secret",
        "refresh-secret",
        "key-secret",
        "client-secret",
        "pass-secret",
        "bearer-secret",
        "user:",
        "oauth-secret",
        "state-secret",
        "fragment-secret",
    ] {
        assert!(!filtered.contains(secret), "credential leaked: {secret}");
    }
    assert_eq!(
        diagnostic_text("API key missing; token expired"),
        "API key missing; token expired"
    );
}

#[test]
fn unicode_controls_and_long_error_chains_have_bounded_output() {
    let mut error = anyhow::Error::msg(format!("\0\r\n{}", "失".repeat(5000)));
    for _ in 0..20 {
        error = error.context("different stage ".repeat(100));
    }
    let message = message(&error);
    assert!(message.chars().count() <= MAX_MESSAGE_CHARS);
    assert!(!message.chars().any(char::is_control));
    assert_eq!(
        super::message(&anyhow::Error::msg("\0\n")),
        "Authentication failed"
    );
}
