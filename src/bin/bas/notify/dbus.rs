use std::collections::HashMap;

use async_trait::async_trait;
use zbus::zvariant::Value;

use super::{Error, Notification, Notifier, Urgency};

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
/// `-1` lets the notification server pick how long to show the notification.
/// See the Desktop Notifications Specification.
const DEFAULT_TIMEOUT: i32 = -1;
/// Application name reported to the notification server.
const DEFAULT_APP_NAME: &str = "bas";

/// Notifier backed by `org.freedesktop.Notifications` over the session bus.
pub struct Dbus {
    connection: zbus::Connection,
    app_name: String,
}

impl Dbus {
    /// Connects to the session bus.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no session bus, which is the normal case
    /// for a headless systemd unit.
    pub async fn connect() -> Result<Self, zbus::Error> {
        let connection = zbus::Connection::session().await?;
        connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "GetServerInformation",
                &(),
            )
            .await?;
        Ok(Self {
            connection,
            app_name: DEFAULT_APP_NAME.to_owned(),
        })
    }
}

#[async_trait]
impl Notifier for Dbus {
    async fn notify(&self, notification: Notification) -> Result<(), Error> {
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("urgency", Value::U8(notification.urgency.into()));
        let actions: Vec<&str> = Vec::new();

        let reply = self
            .connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "Notify",
                &(
                    self.app_name.as_str(),
                    notification.replaces.unwrap_or(0),
                    "",
                    notification.title.as_str(),
                    notification.body.as_str(),
                    actions,
                    hints,
                    DEFAULT_TIMEOUT,
                ),
            )
            .await?;

        // The server returns the id it assigned; nothing consumes it yet, but
        // deserializing proves the call really succeeded.
        reply.body().deserialize::<u32>()?;
        Ok(())
    }
}

impl From<Urgency> for u8 {
    fn from(urgency: Urgency) -> Self {
        match urgency {
            Urgency::Low => 0,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        }
    }
}
