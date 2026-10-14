use std::process::Stdio;

use serde_json::json;
use tokio::{io::AsyncWriteExt, process::Command};

#[tokio::test]
async fn eof_never_records_a_response_but_complete_frames_do() {
    for (mode, method, id, record) in [
        ("agent_auth_form", "authenticate", "elicit", "elicitation"),
        (
            "session_elicitation",
            "session/prompt",
            "session-form",
            "elicitation",
        ),
        (
            "session_async_elicitation",
            "session/prompt",
            "async-form",
            "elicitation",
        ),
        (
            "interactive_permission",
            "session/prompt",
            "permission",
            "permission",
        ),
    ] {
        for send_response in [false, true] {
            let directory = tempfile::tempdir().expect("fixture directory");
            let mut child = Command::new("sh")
                .args(["-c", include_str!("../assets/fixture.sh")])
                .env("BAS_FIXTURE_MODE", mode)
                .env("BAS_FIXTURE_PID", directory.path().join("pid"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .expect("fixture peer");
            let mut input = child.stdin.take().expect("fixture input");
            let request = json!({"jsonrpc":"2.0","id":1,"method":method,"params":{}});
            input
                .write_all(format!("{request}\n").as_bytes())
                .await
                .expect("start peer request");
            let response = json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"action":"accept","content":{"project":"valid"}}
            });
            if send_response {
                input
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .expect("send response");
            }
            drop(input);
            let output = tokio::time::timeout(super::TIMEOUT, child.wait_with_output())
                .await
                .expect("EOF terminates fixture")
                .expect("wait for peer");
            assert!(
                output.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let path = directory.path().join(format!("pid.{record}"));
            if send_response {
                let recorded: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&path).expect("complete response is recorded"),
                )
                .expect("response JSON");
                assert_eq!(recorded, response, "{mode}");
            } else {
                assert!(
                    !path.exists(),
                    "{mode}: EOF must not create a response record"
                );
            }
        }
    }
}
