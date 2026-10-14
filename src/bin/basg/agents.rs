//! Shared agent services, metadata and presentation, independent of settings views.

pub(crate) mod catalog;
pub(crate) mod check;
pub(crate) mod icon;
pub(crate) mod metadata;
mod status;

pub(crate) use status::Status;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

/// Agent changes wake every chat window attached to the same daemon.
#[derive(Clone, Default)]
pub(crate) struct Changes {
    pub server: Option<std::net::SocketAddr>,
    pub ready: Option<String>,
    pub select: Option<String>,
    pub target: Option<gpui_kit::WindowId>,
    pub revision: u64,
}
impl gpui_kit::Global for Changes {}

pub(crate) fn init(
    server: std::net::SocketAddr,
    runtime: tokio::runtime::Handle,
    cx: &mut gpui_kit::App,
) {
    check::init(server, runtime.clone(), cx);
    catalog::init(server, runtime, cx);
    if !cx.has_global::<Changes>() {
        cx.set_global(Changes::default());
    }
}
