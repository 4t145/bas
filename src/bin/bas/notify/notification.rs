use async_trait::async_trait;

/// How insistent a notification is.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// A desktop notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub urgency: Urgency,
    /// Replaces an earlier notification with the same id, when supported.
    pub replaces: Option<u32>,
}

impl Notification {
    /// Builds a normal-urgency notification.
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            urgency: Urgency::default(),
            replaces: None,
        }
    }

    /// Overrides the urgency.
    #[must_use]
    pub fn with_urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }
}

/// Failure raised while delivering a notification.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The session bus is unreachable or refused the call.
    #[error("cannot reach the desktop notification service: {0}")]
    Bus(#[from] zbus::Error),
    /// The notification service answered with something unexpected.
    #[error("the desktop notification service rejected the call: {0}")]
    Service(#[from] zbus::fdo::Error),
}

/// Delivers notifications.
#[async_trait]
pub trait Notifier: Send + Sync {
    /// Shows one notification.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the desktop service is unreachable or refuses the
    /// call.
    async fn notify(&self, notification: Notification) -> Result<(), Error>;
}
