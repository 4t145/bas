use super::*;

#[tokio::test]
async fn binary_file_reads_return_a_text_only_error_without_breaking_the_session() {
    let directory = tempfile::tempdir().expect("temporary files");
    let registry = fixture("file_read", &directory, TIMEOUT);
    let session = registry.create(request()).await.expect("ACP session");
    for (content, expected_text) in [
        (b"\x89PNG\r\n\x1a\nbinary image".as_slice(), None),
        (b"\xff\xd8\xffbinary jpeg".as_slice(), None),
        (
            "A UTF-8 file\n中文\n".as_bytes(),
            Some("A UTF-8 file\n中文\n"),
        ),
    ] {
        std::fs::write(directory.path().join("pid.input"), content).expect("file contents");
        let receipt = session
            .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
                session.id().to_string(),
                vec!["read file".into()],
            )))
            .await
            .expect("read turn");
        let mut events = session
            .try_attach(AttachOptions {
                cursor: Some(receipt.cursor),
                replay: 0,
            })
            .await
            .expect("turn events");
        tokio::time::timeout(TIMEOUT, async {
            loop {
                let event = events.recv().await.expect("event");
                if matches!(event.kind, bas::model::EventKind::TurnFinished { .. }) {
                    break;
                }
            }
        })
        .await
        .expect("file request does not hang the session");
        let response: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("pid.file-read")).expect("recorded response"),
        )
        .expect("JSON response");
        if let Some(expected) = expected_text {
            assert_eq!(response["result"]["content"], expected);
            assert!(response.get("error").is_none());
        } else {
            assert_eq!(
                response["error"]["code"],
                serde_json::to_value(v1::ErrorCode::InvalidParams).expect("error code")
            );
            let message = response["error"]["message"]
                .as_str()
                .expect("error message");
            assert!(message.contains("UTF-8 text"));
            assert!(message.contains("images or other binary files"));
            assert!(response.get("result").is_none(), "never return lossy text");
        }
    }
    registry.shutdown().await;
}
