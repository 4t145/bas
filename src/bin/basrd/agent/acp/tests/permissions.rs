use super::*;
use basr::service::pb::basr_server::Basr;
use basr::{model::PendingPermission, service::pb};
use futures::StreamExt;
use tonic::Request;

#[tokio::test]
async fn native_acp_tool_waits_for_gui_permission_and_receives_the_selected_option() {
    for option in ["allow-once", "reject-once"] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let registry = Arc::new(fixture_with_limits(
            "interactive_permission",
            &directory,
            Limits::default(),
        ));
        let session = registry.create(request()).await.expect("ACP session");
        let service = crate::service::Service::new(registry.clone());
        let mut approvals = service
            .watch_permissions(Request::new(pb::WatchPermissionsRequest {
                session: session.id().to_string(),
            }))
            .await
            .expect("approval subscription")
            .into_inner();
        approvals
            .next()
            .await
            .expect("first snapshot")
            .expect("valid snapshot");
        let receipt = session
            .accept_prompt(v1::PromptRequest::new(
                session.id().to_string(),
                vec!["run pwd".into()],
            ))
            .await
            .expect("start turn");
        let snapshot = tokio::time::timeout(TIMEOUT, approvals.next())
            .await
            .expect("ACP approval deadline")
            .expect("pending snapshot")
            .expect("valid snapshot");
        let permission: PendingPermission = snapshot
            .pending
            .into_iter()
            .next()
            .expect("permission")
            .try_into()
            .expect("ACP request");
        assert_eq!(
            permission.request.session_id.to_string(),
            session.id().to_string()
        );
        assert_eq!(
            permission.request.tool_call.fields.raw_input,
            Some(serde_json::json!({"command":"pwd"}))
        );
        assert_eq!(session.meta().status, SessionStatus::Working);
        assert!(
            !directory.path().join("pid.permission").exists(),
            "ACP cannot proceed before approval"
        );
        let outcome = v1::RequestPermissionResponse::new(v1::RequestPermissionOutcome::Selected(
            v1::SelectedPermissionOutcome::new(option),
        ));
        service
            .respond_permission(Request::new(pb::RespondPermissionRequest {
                session: session.id().to_string(),
                request_id: permission.request_id,
                response: Some((&outcome).into()),
            }))
            .await
            .expect("submit offered option");
        let mut events = session
            .try_attach(AttachOptions {
                cursor: Some(receipt.cursor),
                replay: 0,
            })
            .await
            .expect("turn events");
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if let basr::model::EventKind::TurnFinished { result, .. } =
                    events.recv().await.expect("turn event").kind
                {
                    assert_eq!(
                        result.expect("ACP completion").stop_reason,
                        StopReason::EndTurn
                    );
                    break;
                }
            }
        })
        .await
        .expect("ACP finishes after approval");
        let response: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(directory.path().join("pid.permission"))
                .expect("ACP permission reply"),
        )
        .expect("JSON reply");
        assert_eq!(response["result"]["outcome"]["optionId"], option);
        assert_eq!(response["result"]["outcome"]["outcome"], "selected");
        drop(approvals);
        registry.shutdown().await;
    }
}

#[tokio::test]
async fn agent_cancelled_permission_is_removed_without_cancelling_the_successor_for_the_same_tool()
{
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture_with_limits(
        "permission_cancel_successor",
        &directory,
        Limits::default(),
    ));
    let session = registry.create(request()).await.expect("ACP session");
    let service = crate::service::Service::new(registry.clone());
    let mut approvals = service
        .watch_permissions(Request::new(pb::WatchPermissionsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("approval subscription")
        .into_inner();
    approvals
        .next()
        .await
        .expect("first snapshot")
        .expect("valid snapshot");
    let receipt = session
        .accept_prompt(v1::PromptRequest::new(
            session.id().to_string(),
            vec!["run command".into()],
        ))
        .await
        .expect("start turn");
    let old: PendingPermission = tokio::time::timeout(TIMEOUT, approvals.next())
        .await
        .expect("initial approval deadline")
        .expect("initial approval snapshot")
        .expect("valid snapshot")
        .pending
        .into_iter()
        .next()
        .expect("old approval")
        .try_into()
        .expect("ACP permission");
    assert_eq!(old.request.options[0].option_id.to_string(), "allow-old");
    std::fs::write(directory.path().join("pid.cancel"), "cancel old request")
        .expect("tell agent to cancel the request");

    let successor: PendingPermission = tokio::time::timeout(TIMEOUT, async {
        loop {
            let snapshot = approvals
                .next()
                .await
                .expect("approval stream")
                .expect("valid snapshot");
            if snapshot.pending.len() == 1 {
                let pending: PendingPermission = snapshot
                    .pending
                    .into_iter()
                    .next()
                    .expect("single permission")
                    .try_into()
                    .expect("ACP permission");
                if pending.request.options[0].option_id.to_string() == "allow-successor" {
                    break pending;
                }
            }
        }
    })
    .await
    .expect("cancelled card is removed and the next request is dispatched");
    assert_ne!(successor.request_id, old.request_id);
    assert_eq!(
        successor.request.tool_call.tool_call_id,
        old.request.tool_call.tool_call_id
    );
    let stale = service
        .respond_permission(Request::new(pb::RespondPermissionRequest {
            session: session.id().to_string(),
            request_id: old.request_id,
            response: Some(
                (&v1::RequestPermissionResponse::new(v1::RequestPermissionOutcome::Selected(
                    v1::SelectedPermissionOutcome::new("allow-old"),
                )))
                    .into(),
            ),
        }))
        .await
        .expect_err("old permission is no longer answerable");
    assert_eq!(stale.code(), tonic::Code::FailedPrecondition);
    assert!(
        !directory.path().join("pid.permission-successor").exists(),
        "successor still waits for the user"
    );
    service
        .respond_permission(Request::new(pb::RespondPermissionRequest {
            session: session.id().to_string(),
            request_id: successor.request_id,
            response: Some(
                (&v1::RequestPermissionResponse::new(v1::RequestPermissionOutcome::Selected(
                    v1::SelectedPermissionOutcome::new("allow-successor"),
                )))
                    .into(),
            ),
        }))
        .await
        .expect("answer successor");
    let mut events = session
        .try_attach(AttachOptions {
            cursor: Some(receipt.cursor),
            replay: 0,
        })
        .await
        .expect("turn events");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let basr::model::EventKind::TurnFinished { result, .. } =
                events.recv().await.expect("turn event").kind
            {
                assert_eq!(result.expect("completion").stop_reason, StopReason::EndTurn);
                break;
            }
        }
    })
    .await
    .expect("turn finishes without a stale permission blocking it");
    let cancelled: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("pid.permission-cancelled"))
            .expect("cancelled request reply"),
    )
    .expect("JSON reply");
    assert_eq!(cancelled["error"]["code"], -32800);
    let selected: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("pid.permission-successor"))
            .expect("successor reply"),
    )
    .expect("JSON reply");
    assert_eq!(selected["result"]["outcome"]["optionId"], "allow-successor");
    assert!(
        approvals
            .next()
            .await
            .expect("cleared snapshot")
            .expect("valid snapshot")
            .pending
            .is_empty()
    );
    drop(approvals);
    registry.shutdown().await;
}
