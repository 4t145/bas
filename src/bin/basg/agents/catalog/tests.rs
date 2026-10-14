use super::*;
use bas::config::agent::Command;

fn operation(complete: bool, spec_json: Vec<u8>) -> pb::AgentOperation {
    pb::AgentOperation {
        phase: "ignored transport phase".into(),
        message: "installation progress".into(),
        complete,
        spec_json,
    }
}

#[test]
fn completed_progress_carries_the_decoded_spec() {
    let spec = AgentSpec::acp("fixture", Command::new("/fixture/agent", []));
    let progress = decode(operation(
        true,
        serde_json::to_vec(&spec).expect("fixture spec"),
    ));
    assert!(progress.complete);
    assert_eq!(progress.message, "installation progress");
    assert_eq!(progress.spec, Some(spec));
}

#[test]
fn intermediate_progress_does_not_offer_a_spec() {
    let spec = AgentSpec::acp("fixture", Command::new("/fixture/agent", []));
    for payload in [
        serde_json::to_vec(&spec).expect("fixture spec"),
        b"invalid JSON".to_vec(),
    ] {
        let progress = decode(operation(false, payload));
        assert!(!progress.complete);
        assert_eq!(progress.message, "installation progress");
        assert!(progress.spec.is_none());
    }
}

#[test]
fn malformed_or_absent_spec_does_not_hide_completion() {
    for payload in [Vec::new(), b"invalid JSON".to_vec(), b"{}".to_vec()] {
        let progress = decode(operation(true, payload));
        assert!(progress.complete);
        assert_eq!(progress.message, "installation progress");
        assert!(progress.spec.is_none());
    }
}

#[test]
fn updates_retains_task_ownership_until_consumer_drop() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("fixture runtime");
    runtime.block_on(async {
        let (send, receiver) = mpsc::unbounded();
        let (alive, mut ended) = oneshot::channel::<()>();
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            let _alive = alive;
            send.unbounded_send(Ok(decode(operation(false, Vec::new()))))
                .expect("consumer is alive");
            std::future::pending::<()>().await;
        }));
        let mut updates = Updates {
            receiver,
            _task: task,
        };
        let progress = updates
            .next()
            .await
            .expect("progress event")
            .expect("successful progress");
        assert!(!progress.complete);
        assert!(ended.try_recv().expect("task still alive").is_none());
        drop(updates);
        assert!(
            ended.await.is_err(),
            "dropping the consumer aborts its task"
        );
    });
}
