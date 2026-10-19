//! HTTP transport for GPUI-managed assets, sharing the GUI's network runtime.

use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result};
use futures::future::BoxFuture;
use gpui_kit::http_client::{
    AsyncBody, HttpClient, Request, RequestTimeout, Response, Url, http::HeaderValue,
};
use reqwest_client::ReqwestClient;
use tokio::runtime::Handle;

const USER_AGENT: &str = concat!("basg/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

struct AssetClient {
    inner: ReqwestClient,
}

impl HttpClient for AssetClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        self.inner.user_agent()
    }

    fn proxy(&self) -> Option<&Url> {
        self.inner.proxy()
    }

    /// # Errors
    /// Propagates transport, TLS, and request-deadline failures.
    fn send(
        &self,
        mut request: Request<AsyncBody>,
    ) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        if request.extensions().get::<RequestTimeout>().is_none() {
            request
                .extensions_mut()
                .insert(RequestTimeout(REQUEST_TIMEOUT));
        }
        self.inner.send(request)
    }
}

/// Creates the transport before any GPUI window can request a remote asset.
///
/// # Errors
/// Returns an error if HTTP or TLS client configuration cannot be initialized.
pub(super) fn client(runtime: &Handle) -> Result<Arc<dyn HttpClient>> {
    // The adapter captures the current handle; it can then be polled by GPUI
    // without creating a second Tokio runtime.
    let _entered = runtime.enter();
    // A request deadline is created on Tokio and also bounds body reads.
    // A per-read idle timer would require Tokio context while GPUI polls the body.
    let inner = ReqwestClient::proxy_and_user_agent(None, USER_AGENT)
        .context("cannot initialize the GUI asset HTTP client")?;
    Ok(Arc::new(AssetClient { inner }))
}
