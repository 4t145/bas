use super::*;

#[test]
fn binding_waits_for_queued_and_active_callbacks() {
    let activity = Arc::new(Activity::default());
    activity.observe(
        r#"{"method":"session/update","params":{}}"#,
        LineDirection::Stdout,
    );
    assert!(!activity.try_bind(|| panic!("queued warm traffic")));
    let callback = activity.enter().expect("callback");
    assert!(!activity.try_bind(|| panic!("active warm callback")));
    drop(callback);
    let mut bound = false;
    assert!(activity.try_bind(|| bound = true));
    assert!(bound);
}
use LineDirection::{Stdin, Stdout};

#[test]
fn outbound_does_not_consume_queued_inbound_work() {
    let activity = Arc::new(Activity::default());
    activity.observe(r#"{"method":"session/update"}"#, Stdout);
    let guard = activity.outbound().expect("outbound allowed");
    assert!(!activity.try_claim(Duration::ZERO));
    drop(guard);
    assert!(!activity.try_claim(Duration::ZERO));
    drop(activity.enter().expect("inbound callback"));
    assert!(activity.try_claim(Duration::ZERO));
    assert!(activity.outbound().is_none());
}

#[test]
fn claim_seals_and_rejects_callbacks() {
    let activity = Arc::new(Activity::default());
    assert!(!activity.try_claim(Duration::MAX));
    assert!(activity.try_claim(Duration::ZERO));
    assert!(!activity.try_claim(Duration::ZERO));
    assert!(activity.enter().is_none());
    activity.observe(r#"{"method":"late"}"#, Stdout);
    assert!(activity.enter().is_none());
}

#[test]
fn outgoing_waits_for_matching_response() {
    let activity = Activity::default();
    activity.observe(r#"{"id":1,"method":"session/prompt"}"#, Stdin);
    activity.observe(r#"{"id":"1","result":{}}"#, Stdout);
    assert!(!activity.try_claim(Duration::ZERO));
    activity.observe(r#"{"id":1,"result":{}}"#, Stdin);
    assert!(!activity.try_claim(Duration::ZERO));
    activity.observe(r#"{"id":1,"error":{"code":-1}}"#, Stdout);
    assert!(activity.try_claim(Duration::ZERO));
}

#[test]
fn inbound_waits_for_callback_and_response() {
    let activity = Arc::new(Activity::default());
    activity.observe(r#"{"id":"request","method":"permission"}"#, Stdout);
    assert!(!activity.try_claim(Duration::ZERO));
    let guard = activity.enter().expect("callback allowed");
    drop(guard);
    assert!(!activity.try_claim(Duration::ZERO));
    activity.observe(r#"{"id":"request","result":{}}"#, Stdin);
    assert!(activity.try_claim(Duration::ZERO));
}

#[test]
fn queued_notifications_and_active_callbacks_block_claim() {
    let activity = Arc::new(Activity::default());
    activity.observe(r#"{"method":"session/update"}"#, Stdout);
    activity.observe(r#"{"method":"session/update"}"#, Stdout);
    let first = activity.enter().expect("first callback");
    drop(first);
    assert!(!activity.try_claim(Duration::ZERO));
    let second = activity.enter().expect("second callback");
    assert!(!activity.try_claim(Duration::ZERO));
    drop(second);
    let unqueued = activity.enter().expect("unqueued callback");
    assert!(!activity.try_claim(Duration::ZERO));
    drop(unqueued);
    assert!(activity.try_claim(Duration::ZERO));
}

#[test]
fn every_nonblank_line_and_callback_refreshes_last_activity() {
    let activity = Arc::new(Activity::default());
    let old = Instant::now() - Duration::from_secs(60);
    for (line, direction) in [
        (r#"{"id":"unknown","result":{}}"#, Stdout),
        ("invalid json", Stdout),
        ("diagnostic", LineDirection::Stderr),
        (r#"{"method":"notification"}"#, Stdin),
    ] {
        activity.state.lock().expect("state").last = old;
        activity.observe(line, direction);
        assert!(activity.state.lock().expect("state").last > old);
    }
    activity.state.lock().expect("state").last = old;
    activity.observe(" \t\n", Stdout);
    assert_eq!(activity.state.lock().expect("state").last, old);
    let guard = activity.enter().expect("callback");
    assert!(activity.state.lock().expect("state").last > old);
    activity.state.lock().expect("state").last = old;
    drop(guard);
    assert!(activity.state.lock().expect("state").last > old);
}

fn url_activity(action: &str) -> Arc<Activity> {
    let activity = Arc::new(Activity::default());
    activity.observe(
        r#"{"id":7,"method":"elicitation/create","params":{"mode":"url","elicitationId":"auth"}}"#,
        Stdout,
    );
    drop(activity.enter().expect("create callback"));
    activity.observe(
        &serde_json::json!({"id":7,"result":{"action":action}}).to_string(),
        Stdin,
    );
    activity
}

#[test]
fn accepted_url_waits_for_completion_and_its_callback() {
    let activity = url_activity("accept");
    assert!(!activity.try_claim(Duration::ZERO));
    activity.observe(
        r#"{"method":"elicitation/complete","params":{"elicitationId":"other"}}"#,
        Stdout,
    );
    drop(activity.enter().expect("unrelated completion"));
    assert!(!activity.try_claim(Duration::ZERO));
    activity.observe(
        r#"{"method":"elicitation/complete","params":{"elicitationId":"auth"}}"#,
        Stdout,
    );
    assert!(!activity.try_claim(Duration::ZERO));
    drop(activity.enter().expect("completion callback"));
    assert!(activity.try_claim(Duration::ZERO));
}

#[test]
fn declined_urls_release_but_unknown_actions_are_conservative() {
    for action in ["decline", "cancel"] {
        assert!(url_activity(action).try_claim(Duration::ZERO));
    }
    assert!(!url_activity("unknown").try_claim(Duration::ZERO));
}
