use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use basr::{
    model::{ContentBlock, Cursor, Owner},
    service::pb::{self, basr_server::Basr},
};
use futures::StreamExt;
use tonic::{Code, Request};

use super::Service;
use crate::{
    agent::{Catalog, LocalHost},
    session::{CreateRequest, Limits, Registry},
};

fn registry(history: usize) -> Arc<Registry> {
    Arc::new(Registry::new(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits {
            history_capacity: NonZeroUsize::new(history).expect("nonzero history"),
            ..Limits::default()
        },
    ))
}

async fn create(registry: &Registry) -> Arc<crate::session::Session> {
    registry
        .create(CreateRequest {
            agent: "embedded".into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .expect("create")
}

#[tokio::test]
async fn dropping_an_idle_response_stream_immediately_detaches() {
    let registry = registry(100);
    let session = create(&registry).await;
    let service = Service::new(Arc::clone(&registry));
    let stream = service
        .attach(Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            cursor: None,
            replay: 0,
        }))
        .await
        .expect("attach");
    assert_eq!(session.subscriber_count(), 1);
    drop(stream);
    assert_eq!(session.subscriber_count(), 0);
    registry.shutdown().await;
}

#[tokio::test]
async fn foreground_stream_switches_sessions_and_disconnect_releases_presence() {
    use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
    let registry = registry(100);
    let first = create(&registry).await;
    let second = create(&registry).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(Service::new(registry.clone()).into_server())
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let mut client = basr::service::Client::connect(format!("http://{address}"))
        .await
        .expect("client");
    let (updates, receiver) = tokio::sync::mpsc::channel(4);
    let mut stream = client
        .set_foreground(ReceiverStream::new(receiver))
        .await
        .expect("foreground stream")
        .into_inner();
    for target in [Some(first.id()), Some(second.id()), None, Some(first.id())] {
        updates
            .send(pb::ForegroundRequest {
                session: target.map(|id| id.to_string()),
            })
            .await
            .expect("focus update");
        tokio::time::timeout(Duration::from_secs(5), stream.message())
            .await
            .expect("ack deadline")
            .expect("valid ack")
            .expect("ack");
        assert_eq!(
            first.foreground_viewers(),
            usize::from(target == Some(first.id()))
        );
        assert_eq!(
            second.foreground_viewers(),
            usize::from(target == Some(second.id()))
        );
    }
    drop(stream);
    drop(updates);
    tokio::time::timeout(Duration::from_secs(5), async {
        while first.foreground_viewers() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("disconnect releases foreground state");
    let (updates, receiver) = tokio::sync::mpsc::channel(1);
    updates
        .send(pb::ForegroundRequest {
            session: Some(second.id().to_string()),
        })
        .await
        .expect("foreground update");
    let mut stream = client
        .set_foreground(ReceiverStream::new(receiver))
        .await
        .expect("another viewer")
        .into_inner();
    stream.message().await.expect("ack").expect("active viewer");
    registry.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), stream.message())
            .await
            .expect("shutdown drains idle foreground streams")
            .expect("closed stream")
            .is_none()
    );
    assert_eq!(second.foreground_viewers(), 0);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn grpc_resume_delivers_every_retained_page_before_live_events() {
    let registry = registry(100);
    let session = create(&registry).await;
    for i in 0..10 {
        session.set_title(&format!("{i}")).expect("rename");
    }
    let service = Service::new(Arc::clone(&registry));
    let mut stream = service
        .attach(Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            cursor: Some(0),
            replay: 2,
        }))
        .await
        .expect("attach")
        .into_inner();
    for cursor in 1..=11 {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await
                .expect("no missing page")
                .expect("event")
                .expect("success")
                .cursor,
            cursor
        );
    }
    registry.shutdown().await;
    stream.next().await.expect("stop event").expect("success");
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn grpc_reports_history_gaps_invalid_cursors_busy_and_missing_sessions() {
    let registry = registry(3);
    let session = create(&registry).await;
    let service = Service::new(Arc::clone(&registry));
    for i in 0..10 {
        session.set_title(&format!("{i}")).expect("rename");
    }
    let error = service
        .attach(Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            cursor: Some(0),
            replay: 1,
        }))
        .await
        .err()
        .expect("expired cursor");
    assert_eq!(error.code(), Code::DataLoss);
    let error = service
        .load_history(Request::new(pb::LoadHistoryRequest {
            session: session.id().to_string(),
            after: Some(1000),
            limit: 1,
        }))
        .await
        .expect_err("future cursor");
    assert_eq!(error.code(), Code::InvalidArgument);
    let page = service
        .load_history(Request::new(pb::LoadHistoryRequest {
            session: session.id().to_string(),
            after: None,
            limit: 0,
        }))
        .await
        .expect("default page size")
        .into_inner();
    assert_eq!(page.events.len(), 3);
    session
        .prompt(vec![ContentBlock::from("long prompt".repeat(100))])
        .expect("first prompt");
    let error = service
        .prompt(Request::new(pb::AcpPayload::from(
            &basr::acp::PromptRequest::new(
                session.id().to_string(),
                vec![basr::acp::ContentBlock::from("second")],
            ),
        )))
        .await
        .expect_err("busy");
    assert_eq!(error.code(), Code::FailedPrecondition);
    registry.remove(session.id()).await.expect("remove");
    let error = service
        .delete_session(Request::new(pb::DeleteSessionRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect_err("already removed");
    assert_eq!(error.code(), Code::NotFound);
    assert!(session.page(Some(Cursor::from_raw(10000)), 1).is_err());
}
