//! The stable v1 fallback emits one plan and clears it when work is cancelled.
mod support;

use agent_client_protocol::{
    Client, Error,
    schema::{ProtocolVersion, v1},
};
use tokio::{sync::mpsc::unbounded_channel, time::timeout};

#[tokio::test]
async fn cancelling_a_legacy_plan_clears_it_without_sending_experimental_updates()
-> Result<(), Box<dyn std::error::Error>> {
    let (send, mut receive) = unbounded_channel();
    Client
        .builder()
        .on_receive_notification(
            async move |notification: v1::SessionNotification, _| {
                send.send(notification.update)
                    .map_err(Error::into_internal_error)
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(support::agent(1), async move |cx| {
            cx.send_request(v1::InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let session = cx
                .send_request(v1::NewSessionRequest::new(support::cwd()))
                .block_task()
                .await?
                .session_id;
            let prompt_cx = cx.clone();
            let prompt_id = session.clone();
            let pending = tokio::spawn(async move {
                prompt_cx
                    .send_request(v1::PromptRequest::new(prompt_id, vec!["/plan 5000".into()]))
                    .block_task()
                    .await
            });
            loop {
                let update = timeout(support::UPDATE_TIMEOUT, receive.recv())
                    .await
                    .expect("plan in time")
                    .expect("connected");
                assert!(!matches!(
                    update,
                    v1::SessionUpdate::PlanUpdate(_) | v1::SessionUpdate::PlanRemoved(_)
                ));
                if let v1::SessionUpdate::Plan(plan) = update {
                    assert_eq!(plan.entries.len(), 3);
                    break;
                }
            }
            cx.send_notification(v1::CancelNotification::new(session))?;
            let response = timeout(support::UPDATE_TIMEOUT, pending)
                .await
                .expect("cancel in time")
                .expect("worker")?;
            assert_eq!(response.stop_reason, v1::StopReason::Cancelled);
            let update = timeout(support::UPDATE_TIMEOUT, receive.recv())
                .await
                .expect("clear in time")
                .expect("connected");
            assert!(matches!(update, v1::SessionUpdate::Plan(plan) if plan.entries.is_empty()));
            Ok(())
        })
        .await?;
    Ok(())
}
