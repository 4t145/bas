use super::{Notification, Urgency};

#[test]
fn notifications_default_to_normal_urgency() {
    let notification = Notification::new("title", "body");

    assert_eq!(notification.urgency, Urgency::Normal);
    assert_eq!(notification.replaces, None);
}

#[test]
fn urgency_uses_the_specification_hint_names() {
    assert_eq!(Urgency::Low.as_hint(), "low");
    assert_eq!(Urgency::Normal.as_hint(), "normal");
    assert_eq!(Urgency::Critical.as_hint(), "critical");
}

#[test]
fn builder_overrides_urgency() {
    let notification = Notification::new("t", "b").with_urgency(Urgency::Critical);

    assert_eq!(notification.urgency, Urgency::Critical);
}
