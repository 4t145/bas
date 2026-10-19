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
    spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
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

fn start(spec: &AgentSpec) -> bas::service::pb::AuthenticateAgentRequest {
    bas::service::pb::AuthenticateAgentRequest {
        agent: spec.id.to_string(),
        expected_spec_json: serde_json::to_vec(spec).expect("spec JSON"),
        method_id: "login".into(),
        expected_method_json: Vec::new(),
    }
}

#[tokio::test]
async fn interactive_form_preserves_schema_and_returns_validated_input() {
    use bas::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(async move {
        super::interactive(&spec, None, &start, events, input, TIMEOUT).await
    });
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
async fn invalid_form_response_cancels_without_sending_unvalidated_content() {
    use bas::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_form", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(async move {
        super::interactive(&spec, None, &start, events, input, TIMEOUT).await
    });
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
    wait_for_exit(&directory.path().join("pid")).await;
    assert!(
        !directory.path().join("pid.elicitation").exists(),
        "invalid input reached the peer: {:?}",
        std::fs::read_to_string(directory.path().join("pid.elicitation"))
    );
}

#[tokio::test]
async fn terminal_failure_returns_its_exit_status_without_retrying_authentication() {
    use bas::service::pb;
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture("agent_auth_terminal", &directory);
    let start = start(&spec);
    let (events, mut receive) = tokio::sync::mpsc::channel(8);
    let (controls, input) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(async move {
        super::interactive(&spec, None, &start, events, input, TIMEOUT).await
    });
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
