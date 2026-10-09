use std::{path::Path, time::Duration};

use nix::{sys::signal::kill, unistd::Pid};
use serde_json::Value;

use super::{CheckResult, authenticate};
use crate::agent::{AgentSpec, Command};

const TIMEOUT: Duration = Duration::from_secs(5);

fn fixture(mode: &str, directory: &tempfile::TempDir) -> AgentSpec {
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "sh",
            [
                "-c".to_owned(),
                include_str!("../assets/fixture.sh").to_owned(),
            ],
        ),
    );
    spec.env.insert("BASR_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BASR_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    spec
}

fn recorded(directory: &tempfile::TempDir, name: &str) -> Value {
    let path = directory.path().join(format!("pid.{name}"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("recorded ACP request"))
        .expect("valid ACP request")
}

async fn wait_for_exit(path: &Path) {
    let pid: i32 = std::fs::read_to_string(path)
        .expect("child pid")
        .trim()
        .parse()
        .expect("numeric child pid");
    tokio::time::timeout(TIMEOUT, async {
        while kill(Pid::from_raw(pid), None).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned ACP process exits");
}

async fn wait_for_descendant_exit(path: &Path) {
    let pid: i32 = std::fs::read_to_string(path)
        .expect("descendant pid")
        .trim()
        .parse()
        .expect("numeric descendant pid");
    tokio::time::timeout(TIMEOUT, async {
        while kill(Pid::from_raw(pid), None).is_ok() {
            // Container PID 1 can retain killed orphans; the SDK cannot reap
            // descendants it did not spawn. A zombie is no longer executing.
            #[cfg(target_os = "linux")]
            if std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|status| {
                    status
                        .rsplit_once(')')
                        .and_then(|(_, fields)| fields.split_whitespace().next())
                        .map(|state| state == "Z")
                })
                == Some(true)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned ACP descendant stops executing");
}

#[tokio::test]
async fn agent_authentication_uses_the_same_connection_and_checks_readiness() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let result = authenticate(&fixture("agent_auth", &directory), "login", TIMEOUT)
        .await
        .expect("successful authentication");
    assert!(matches!(result, CheckResult::Connected(_)));
    assert_eq!(
        recorded(&directory, "initialize")["params"]["clientCapabilities"]["auth"]["terminal"],
        true
    );
    assert_eq!(
        recorded(&directory, "authenticate")["params"],
        serde_json::json!({"methodId": "login"})
    );
    let request = recorded(&directory, "new");
    let cwd = request["params"]["cwd"].as_str().expect("isolated cwd");
    assert!(!Path::new(cwd).exists(), "temporary workspace is removed");
    assert!(!directory.path().join("pid.prompt").exists());
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn rejected_authentication_does_not_attempt_session_creation() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let error = authenticate(&fixture("agent_auth_reject", &directory), "login", TIMEOUT)
        .await
        .err()
        .expect("rejected authentication");
    assert_eq!(error.to_string(), "Agent authentication failed");
    assert!(directory.path().join("pid.authenticate").exists());
    assert!(
        directory.path().join("pid.new").exists(),
        "discovery checks current authentication requirements"
    );
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn unadvertised_and_terminal_methods_are_not_sent_to_authenticate() {
    for (mode, method, expected) in [
        (
            "agent_auth",
            "unadvertised",
            "Selected authentication method is not advertised by this agent",
        ),
        (
            "agent_auth_terminal",
            "login",
            "Terminal authentication requires an interactive terminal",
        ),
    ] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let error = authenticate(&fixture(mode, &directory), method, TIMEOUT)
            .await
            .err()
            .expect("unusable authentication method");
        assert_eq!(error.to_string(), expected);
        assert!(!directory.path().join("pid.authenticate").exists());
        assert!(
            directory.path().join("pid.new").exists(),
            "discovery checks current authentication requirements"
        );
        wait_for_exit(&directory.path().join("pid")).await;
    }
}

#[tokio::test]
async fn successful_authenticate_response_does_not_imply_session_readiness() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let result = authenticate(
        &fixture("agent_auth_still_required", &directory),
        "login",
        TIMEOUT,
    )
    .await
    .expect("authentication readiness result");
    let CheckResult::AuthenticationRequired(initialization, error) = result else {
        panic!("agent incorrectly reported as ready");
    };
    assert_eq!(initialization["authMethods"][0]["id"], "login");
    assert_eq!(error.code, agent_client_protocol::ErrorCode::AuthRequired);
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn authentication_denies_files_terminal_and_permissions() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let result = authenticate(&fixture("agent_auth_access", &directory), "login", TIMEOUT)
        .await
        .expect("callbacks are denied without breaking authentication");
    assert!(matches!(result, CheckResult::Connected(_)));
    assert!(!directory.path().join("pid.write").exists());
    assert!(!directory.path().join("pid.prompt").exists());
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn authentication_timeout_releases_the_owned_process_group() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let error = authenticate(
        &fixture("agent_auth_hang", &directory),
        "login",
        Duration::from_millis(250),
    )
    .await
    .err()
    .expect("authentication timeout");
    assert_eq!(error.to_string(), "ACP authentication timed out");
    assert!(directory.path().join("pid.new").exists());
    wait_for_exit(&directory.path().join("pid")).await;
    wait_for_descendant_exit(&directory.path().join("pid.auth-child")).await;
}

#[tokio::test]
async fn cancelling_authentication_releases_the_owned_process_group() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_hang", &directory);
    let operation = tokio::spawn(async move { authenticate(&spec, "login", TIMEOUT).await });
    tokio::time::timeout(TIMEOUT, async {
        while !directory.path().join("pid.auth-child").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("authentication starts");
    operation.abort();
    assert!(
        operation
            .await
            .err()
            .expect("cancelled operation")
            .is_cancelled()
    );
    assert!(directory.path().join("pid.new").exists());
    wait_for_exit(&directory.path().join("pid")).await;
    wait_for_descendant_exit(&directory.path().join("pid.auth-child")).await;
}

fn start(spec: &AgentSpec) -> basr::service::pb::AuthenticateAgentRequest {
    basr::service::pb::AuthenticateAgentRequest {
        agent: spec.id.to_string(),
        expected_spec_json: serde_json::to_vec(spec).expect("spec JSON"),
        method_id: "login".into(),
        expected_method_json: Vec::new(),
    }
}

#[tokio::test]
async fn interactive_form_preserves_schema_and_returns_validated_input() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("form event timeout")
        .expect("event")
        .expect("valid event");
    let Some(pb::authentication_event::Kind::Elicitation(pending)) = event.kind else {
        panic!("expected form");
    };
    let raw: Value = pending
        .request
        .expect("raw request")
        .decode()
        .expect("JSON");
    assert_eq!(raw["_meta"]["futureHint"], "preserved");
    assert_eq!(
        raw["requestedSchema"]["required"],
        serde_json::json!(["project"])
    );
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::ElicitationResponse(
                pb::ElicitationAnswer {
                    request_id: pending.request_id,
                    response: Some(
                        (&serde_json::json!({"action":"accept","content":{"project":"atlas"}}))
                            .into(),
                    ),
                },
            )),
        })
        .await
        .expect("response");
    assert!(matches!(
        task.await.expect("task").expect("authentication"),
        CheckResult::Connected(_)
    ));
    assert_eq!(
        recorded(&directory, "elicitation")["result"]["content"]["project"],
        "atlas"
    );
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn interactive_url_completion_is_connection_scoped_and_keeps_metadata() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_url", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("URL event timeout")
        .expect("event")
        .expect("valid event");
    let Some(pb::authentication_event::Kind::Elicitation(pending)) = event.kind else {
        panic!("expected URL request");
    };
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::ElicitationResponse(
                pb::ElicitationAnswer {
                    request_id: pending.request_id,
                    response: Some((&serde_json::json!({"action":"accept"})).into()),
                },
            )),
        })
        .await
        .expect("response");
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("completion timeout")
        .expect("completion")
        .expect("valid completion");
    let Some(pb::authentication_event::Kind::ElicitationComplete(payload)) = event.kind else {
        panic!("expected completion");
    };
    let raw: Value = payload.decode().expect("completion JSON");
    assert_eq!(raw["elicitationId"], "opaque/url:17");
    assert_eq!(raw["_meta"]["future"], "preserved");
    assert!(matches!(
        task.await.expect("task").expect("authentication"),
        CheckResult::Connected(_)
    ));
    assert!(
        receive.try_recv().is_err(),
        "unrecognized completion is ignored"
    );
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn invalid_form_response_cancels_without_sending_unvalidated_content() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("form timeout")
        .expect("event")
        .expect("valid event");
    let Some(pb::authentication_event::Kind::Elicitation(pending)) = event.kind else {
        panic!("expected form");
    };
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::ElicitationResponse(
                pb::ElicitationAnswer {
                    request_id: pending.request_id,
                    response: Some(
                        (&serde_json::json!({"action":"accept","content":{"project":7}})).into(),
                    ),
                },
            )),
        })
        .await
        .expect("response");
    assert!(task.await.expect("task").is_err());
    assert!(!directory.path().join("pid.elicitation").exists());
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn disconnect_during_form_releases_connection_and_pending_responder() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (_controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("form timeout")
        .expect("event")
        .expect("valid event");
    drop(receive);
    let error = task.await.expect("task").err().expect("cancelled");
    assert_eq!(
        error.to_string(),
        "Interactive authentication was cancelled"
    );
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn stale_method_and_unknown_type_are_not_executed() {
    use basr::service::pb;
    for mode in ["agent_auth", "agent_auth_unknown"] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let spec = fixture(mode, &directory);
        let mut start = start(&spec);
        if mode == "agent_auth" {
            start.expected_method_json =
                serde_json::to_vec(&serde_json::json!({"id":"login","name":"Old method"}))
                    .expect("JSON");
        }
        let (events, _receive) =
            tokio::sync::mpsc::channel::<Result<pb::AuthenticationEvent, tonic::Status>>(8);
        let (_controls, input) = tokio::sync::mpsc::channel(8);
        assert!(
            super::interactive(&spec, &start, events, input, TIMEOUT)
                .await
                .is_err()
        );
        assert!(!directory.path().join("pid.authenticate").exists());
        wait_for_exit(&directory.path().join("pid")).await;
    }
}

#[tokio::test]
async fn probe_retains_unknown_authentication_fields_before_sdk_projection() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let CheckResult::AuthenticationRequired(raw, error) =
        super::super::check_connection(&fixture("agent_auth_unknown", &directory), TIMEOUT)
            .await
            .expect("probe")
    else {
        panic!("expected setup requirement");
    };
    assert_eq!(raw["futureInitialize"], 17);
    assert_eq!(raw["authMethods"][0]["type"], "future_mode");
    assert_eq!(raw["authMethods"][0]["futureField"]["kept"], true);
    assert_eq!(error.raw["futureError"]["preserved"], true);
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn terminal_plan_closes_acp_connection_then_rechecks_after_success() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_terminal_ready", &directory);
    let base_args = spec.command().expect("command").args.clone();
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("terminal timeout")
        .expect("event")
        .expect("valid event");
    let Some(pb::authentication_event::Kind::Terminal(payload)) = event.kind else {
        panic!("expected terminal");
    };
    let plan: basr::authentication::Invocation = payload.decode().expect("literal invocation");
    assert_eq!(&plan.args[..base_args.len()], base_args);
    assert_eq!(&plan.args[base_args.len()..], ["login"]);
    wait_for_exit(&directory.path().join("pid")).await;
    std::fs::write(directory.path().join("pid.ready"), "ready")
        .expect("simulate terminal completion");
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::TerminalResult(
                pb::TerminalAuthenticationResult { exit_code: 0 },
            )),
        })
        .await
        .expect("terminal result");
    assert!(matches!(
        task.await.expect("task").expect("readiness check"),
        CheckResult::Connected(_)
    ));
    assert!(
        !directory.path().join("pid.authenticate").exists(),
        "terminal methods are never sent to authenticate RPC"
    );
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn terminal_failure_returns_its_exit_status_without_retrying_authentication() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_terminal", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("terminal timeout")
        .expect("event")
        .expect("valid event");
    assert!(matches!(
        event.kind,
        Some(pb::authentication_event::Kind::Terminal(_))
    ));
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::TerminalResult(
                pb::TerminalAuthenticationResult { exit_code: 23 },
            )),
        })
        .await
        .expect("terminal result");
    assert_eq!(
        task.await
            .expect("task")
            .err()
            .expect("failed terminal")
            .to_string(),
        "Terminal authentication exited with status 23"
    );
    assert!(!directory.path().join("pid.authenticate").exists());
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn agent_cancellation_removes_the_form_and_keeps_the_connection_usable() {
    use basr::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form_cancel", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (_controls, input) = tokio::sync::mpsc::channel(8);
    let task =
        tokio::spawn(
            async move { super::interactive(&spec, &start, events, input, TIMEOUT).await },
        );
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("form timeout")
        .expect("event")
        .expect("valid event");
    let Some(pb::authentication_event::Kind::Elicitation(pending)) = event.kind else {
        panic!("expected form");
    };
    let event = tokio::time::timeout(TIMEOUT, receive.recv())
        .await
        .expect("cancellation timeout")
        .expect("event")
        .expect("valid event");
    assert_eq!(
        event.kind,
        Some(pb::authentication_event::Kind::ElicitationCancelled(
            pending.request_id
        ))
    );
    assert!(matches!(
        task.await
            .expect("task")
            .expect("connection remains usable"),
        CheckResult::Connected(_)
    ));
    assert_eq!(recorded(&directory, "elicitation")["error"]["code"], -32800);
    wait_for_exit(&directory.path().join("pid")).await;
}

#[tokio::test]
async fn unmatched_request_and_session_scopes_are_rejected_before_display() {
    for mode in [
        "agent_auth_form_wrong_scope",
        "agent_auth_form_wrong_session",
    ] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let spec = fixture(mode, &directory);
        let start = start(&spec);
        let (events, mut receive) = tokio::sync::mpsc::channel(8);
        let (_controls, input) = tokio::sync::mpsc::channel(8);
        let error = super::interactive(&spec, &start, events, input, TIMEOUT)
            .await
            .err()
            .expect("invalid scope");
        assert_eq!(
            error.to_string(),
            "Agent authentication failed",
            "Agent receives a request error rather than losing its connection"
        );
        assert_eq!(recorded(&directory, "elicitation")["error"]["code"], -32602);
        assert!(
            receive.try_recv().is_err(),
            "unmatched scope never reaches frontend"
        );
        wait_for_exit(&directory.path().join("pid")).await;
    }
}

#[tokio::test]
async fn strict_session_schema_is_sent_during_discovery_and_after_authentication() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let result = authenticate(&fixture("agent_auth_strict", &directory), "login", TIMEOUT)
        .await
        .expect("strict standard ACP authentication");
    assert!(matches!(result, CheckResult::Connected(_)));
    for attempt in [1, 2] {
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(directory.path().join(format!("pid.new.{attempt}")))
                .expect("session request"),
        )
        .expect("JSON");
        assert_eq!(raw["params"]["mcpServers"], serde_json::json!([]));
        assert!(raw["params"]["cwd"].as_str().is_some());
    }
    assert_eq!(
        recorded(&directory, "initialize")["params"]["clientCapabilities"]["auth"]["terminal"],
        true
    );
    assert_eq!(
        recorded(&directory, "authenticate")["params"],
        serde_json::json!({"methodId":"login"})
    );
    wait_for_exit(&directory.path().join("pid")).await;
}
