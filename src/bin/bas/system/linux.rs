use sd_notify::NotifyState;
use tokio_util::task::AbortOnDropHandle;

const WATCHDOG_PING_DIVISOR: u32 = 2;

pub(super) fn ready() {
    notify(&[NotifyState::Ready]);
}
pub(super) fn stopping() {
    notify(&[NotifyState::Stopping]);
}

pub(super) fn spawn_watchdog() -> Option<AbortOnDropHandle<()>> {
    let ping = sd_notify::watchdog_enabled()? / WATCHDOG_PING_DIVISOR;
    if ping.is_zero() {
        return None;
    }
    Some(AbortOnDropHandle::new(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(ping);
        loop {
            ticker.tick().await;
            notify(&[NotifyState::Watchdog]);
        }
    })))
}

fn notify(states: &[NotifyState]) {
    if let Err(error) = sd_notify::notify(states) {
        tracing::debug!(%error, "cannot notify the service manager");
    }
}
