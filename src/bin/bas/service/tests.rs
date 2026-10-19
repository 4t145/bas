use std::{num::NonZeroUsize, sync::Arc, time::Duration};

mod archive;
#[cfg(unix)]
mod authentication;
#[cfg(unix)]
mod directories;
#[cfg(unix)]
mod logout;
mod management;
mod permissions;
#[cfg(unix)]
mod subagents;
mod submission;

use bas::{
    model::{ContentBlock, Cursor, Owner},
    service::pb::{self, bas_server::Bas},
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
async fn grpc_resume_delivers_every_retained_page_before_live_events() {
    let registry = registry(100);
    let session = create(&registry).await;
    for i in 0..10 {
        session.set_title(&format!("{i}")).await.expect("rename");
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
        session.set_title(&format!("{i}")).await.expect("rename");
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
        .await
        .expect("first prompt");
    let error = service
        .prompt(Request::new(pb::PromptSubmission::from(
            &bas::model::PromptSubmission::new(bas::acp::PromptRequest::new(
                session.id().to_string(),
                vec![bas::acp::ContentBlock::from("second")],
            )),
        )))
        .await
        .expect("explicit rejection")
        .into_inner();
    assert!(
        matches!(error.outcome, Some(pb::prompt_submission_response::Outcome::Rejected(rejection))
        if rejection.kind == pb::PromptRejectionKind::Busy as i32)
    );
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
