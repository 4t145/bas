use super::*;

fn form(session: SessionId) -> Value {
    json!({"sessionId":session.to_string(),"mode":"form","message":"Choose a project name","requestedSchema":{"type":"object","properties":{"project":{"type":"string","minLength":1}},"required":["project"]}})
}

async fn pending(subscription: &mut Subscription) -> Pending {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(pending) = subscription
                .snapshots
                .borrow_and_update()
                .pending
                .first()
                .cloned()
            {
                return pending;
            }
            subscription
                .snapshots
                .changed()
                .await
                .expect("snapshot publisher");
        }
    })
    .await
    .expect("snapshot deadline")
}

fn open(interactions: &Elicitations, session: SessionId) -> uuid::Uuid {
    let connection = uuid::Uuid::new_v4();
    interactions.open_connection(session, connection);
    connection
}

fn url(session: SessionId) -> Value {
    json!({"sessionId":session.to_string(),"mode":"url","message":"Authorize account","url":"https://agent.example.com/auth","elicitationId":"opaque-url"})
}

#[tokio::test]
async fn invalid_form_response_keeps_the_request_available_for_correction() {
    let interactions = Arc::new(Elicitations::default());
    let session = SessionId::new();
    let connection = open(&interactions, session);
    let mut subscription = interactions.subscribe(session);
    let worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move {
            interactions
                .request(session, connection, form(session))
                .await
        })
    };
    let request = pending(&mut subscription).await;
    assert!(
        interactions
            .respond(
                session,
                &request.request_id,
                json!({"action":"accept","content":{}})
            )
            .is_err()
    );
    assert_eq!(subscription.snapshots.borrow().pending.len(), 1);
    interactions
        .respond(
            session,
            &request.request_id,
            json!({"action":"accept","content":{"project":"bas"}}),
        )
        .expect("corrected response");
    assert_eq!(
        worker.await.expect("worker").expect("form result"),
        json!({"action":"accept","content":{"project":"bas"}})
    );
    assert!(subscription.snapshots.borrow().pending.is_empty());
    assert!(
        interactions
            .respond(session, &request.request_id, json!({"action":"cancel"}))
            .is_err()
    );
}

#[tokio::test]
async fn replacing_a_connection_cancels_its_request_and_late_cleanup_preserves_the_new_request() {
    let interactions = Arc::new(Elicitations::default());
    let session = SessionId::new();
    let old = open(&interactions, session);
    let mut subscription = interactions.subscribe(session);
    let old_worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move { interactions.request(session, old, form(session)).await })
    };
    let old_request = pending(&mut subscription).await;
    let current = open(&interactions, session);
    let current_worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move { interactions.request(session, current, form(session)).await })
    };
    let current_request = pending(&mut subscription).await;
    assert_ne!(old_request.request_id, current_request.request_id);
    assert_eq!(
        old_worker.await.expect("old worker").expect("old result"),
        json!({"action":"cancel"})
    );
    interactions.cancel_connection(session, old);
    assert_eq!(
        subscription.snapshots.borrow().pending[0].request_id,
        current_request.request_id
    );
    assert!(
        interactions
            .respond(session, &old_request.request_id, json!({"action":"cancel"}))
            .is_err()
    );
    assert_eq!(
        interactions
            .request(session, old, form(session))
            .await
            .expect("stale request"),
        json!({"action":"cancel"})
    );
    interactions
        .respond(
            session,
            &current_request.request_id,
            json!({"action":"accept","content":{"project":"bas"}}),
        )
        .expect("current answer");
    assert_eq!(
        current_worker
            .await
            .expect("current worker")
            .expect("current result")["action"],
        "accept"
    );
}

#[tokio::test]
async fn old_completion_and_connection_drop_cannot_complete_or_cancel_a_reused_url_identifier() {
    let interactions = Arc::new(Elicitations::default());
    let session = SessionId::new();
    let old = open(&interactions, session);
    let mut subscription = interactions.subscribe(session);
    let worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move { interactions.request(session, old, url(session)).await })
    };
    let old_request = pending(&mut subscription).await;
    interactions
        .respond(session, &old_request.request_id, json!({"action":"accept"}))
        .expect("old consent");
    worker.await.expect("worker").expect("old URL response");
    let current = open(&interactions, session);
    let worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move { interactions.request(session, current, url(session)).await })
    };
    let request = pending(&mut subscription).await;
    interactions
        .respond(session, &request.request_id, json!({"action":"accept"}))
        .expect("current consent");
    worker.await.expect("worker").expect("current URL response");
    interactions.complete(session, old, "opaque-url");
    interactions.cancel_connection(session, old);
    assert_eq!(
        subscription.snapshots.borrow().pending[0].request_id,
        request.request_id
    );
    assert!(subscription.snapshots.borrow().completed_ids.is_empty());
    interactions.complete(session, current, "opaque-url");
    assert!(subscription.snapshots.borrow().pending.is_empty());
    assert_eq!(
        subscription.snapshots.borrow().completed_ids,
        [request.request_id]
    );
}

#[tokio::test]
async fn closing_the_current_connection_cancels_requests_and_requires_new_registration() {
    let interactions = Arc::new(Elicitations::default());
    let session = SessionId::new();
    let connection = open(&interactions, session);
    let mut subscription = interactions.subscribe(session);
    let worker = {
        let interactions = interactions.clone();
        tokio::spawn(async move {
            interactions
                .request(session, connection, form(session))
                .await
        })
    };
    let _ = pending(&mut subscription).await;
    interactions.cancel_connection(session, connection);
    assert_eq!(
        worker.await.expect("worker").expect("cancelled response"),
        json!({"action":"cancel"})
    );
    assert_eq!(
        interactions
            .request(session, connection, form(session))
            .await
            .expect("closed request"),
        json!({"action":"cancel"})
    );
}
