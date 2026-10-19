//! Real agent artwork shared by the registry and conversation rows.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, bail};
use futures::{AsyncReadExt as _, FutureExt as _, future::Either};
use gds_uim::el;
use gpui_kit::{
    App, AssetSource, Global, Hsla, Image, ImageFormat, IntoElement, ParentElement, Pixels,
    RenderImage, RenderOnce, SharedString, Styled, SvgRenderer, Task, TestSupportExt, Window,
    assets::IconName,
    component::{ActiveTheme as _, Icon},
    div,
    http_client::{AsyncBody, HttpClient, RedirectPolicy, Request, RequestTimeout},
    img,
    prelude::*,
    px, svg,
};

const DEFAULT_SIZE: f32 = 24.;
const ICON_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
enum Artwork {
    Svg { bytes: Arc<[u8]>, embedded: bool },
    Bitmap(Arc<RenderImage>),
}

enum Load {
    Loading { _task: Task<()> },
    Ready(Artwork),
    Failed,
}

#[derive(Default)]
struct Cache(HashMap<SharedString, Load>);
impl Global for Cache {}

#[cfg(all(test, feature = "gui-tests"))]
#[derive(Default)]
struct RenderedSources(HashMap<SharedString, SharedString>);
#[cfg(all(test, feature = "gui-tests"))]
impl Global for RenderedSources {}

/// Displays the agent's SVG as a themed alpha mask, or its original bitmap.
#[derive(IntoElement)]
pub(crate) struct AgentIcon {
    id: SharedString,
    uri: SharedString,
    size: Pixels,
    color: Option<Hsla>,
}

impl AgentIcon {
    pub(crate) fn new(id: impl Into<SharedString>, uri: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            uri: uri.into(),
            size: px(DEFAULT_SIZE),
            color: None,
        }
    }

    pub(crate) fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub(crate) fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for AgentIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        #[cfg(all(test, feature = "gui-tests"))]
        {
            if cx.try_global::<RenderedSources>().is_none() {
                cx.set_global(RenderedSources::default());
            }
            cx.global_mut::<RenderedSources>()
                .0
                .insert(self.id.clone(), self.uri.clone());
        }
        let artwork = cached(&self.uri, cx);
        let color = self.color.unwrap_or_else(|| cx.theme().foreground);
        let (kind, content) = match artwork {
            Some(Artwork::Svg { bytes, embedded }) => {
                let logo = if embedded {
                    el! { #svg { path: $self.uri.clone(); } }
                } else {
                    el! { #svg { data: $&bytes; } }
                };
                (
                    "svg",
                    el! { $logo { size: $self.size; text_color: color; } }.into_any_element(),
                )
            }
            Some(Artwork::Bitmap(image)) => (
                "image",
                el! { #img(image) { size: $self.size; } }.into_any_element(),
            ),
            None => (
                "fallback",
                el! {
                    Icon(IconName::Bot) {
                        size: $self.size;
                        text_color: color;
                    }
                }
                .into_any_element(),
            ),
        };
        el! {
            #div {
                id: $self.id.clone();
                test_support;
                size: $self.size;
                flex_shrink_0;
            } [
                #div {
                    id: $format!("{}-{kind}", self.id);
                    test_support;
                    size: $self.size;
                } [$content],
            ]
        }
    }
}

fn cached(uri: &SharedString, cx: &mut App) -> Option<Artwork> {
    if uri.is_empty() {
        return None;
    }
    if cx.try_global::<Cache>().is_none() {
        cx.set_global(Cache::default());
    }
    if let Some(load) = cx.global::<Cache>().0.get(uri) {
        return match load {
            Load::Ready(artwork) => Some(artwork.clone()),
            _ => None,
        };
    }
    let client = cx.http_client();
    let renderer = cx.svg_renderer();
    let assets = cx.asset_source().clone();
    let executor = cx.background_executor().clone();
    let timeout = executor.timer(ICON_TIMEOUT);
    let key = uri.clone();
    let address = uri.clone();
    let loading = executor.spawn(async move {
        match futures::future::select(
            load(address.as_ref(), client, renderer, assets).boxed(),
            timeout.boxed(),
        )
        .await
        {
            Either::Left((result, _)) => result,
            Either::Right(_) => Err(anyhow::anyhow!("agent icon request timed out")),
        }
    });
    let task = cx.spawn(async move |cx| {
        let result = loading.await;
        cx.update(|cx| {
            let state = match result {
                Ok(artwork) => Load::Ready(artwork),
                Err(error) => {
                    tracing::debug!(uri = %key, %error, "agent icon unavailable");
                    Load::Failed
                }
            };
            cx.global_mut::<Cache>().0.insert(key, state);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<Cache>()
        .0
        .insert(uri.clone(), Load::Loading { _task: task });
    None
}

/// # Errors
/// Reports unavailable, oversized or malformed icon artwork.
async fn load(
    uri: &str,
    client: Arc<dyn HttpClient>,
    renderer: SvgRenderer,
    assets: Arc<dyn AssetSource>,
) -> Result<Artwork> {
    if !uri.starts_with("https://") && !uri.starts_with("http://") {
        let bytes = assets.load(uri)?.context("embedded agent icon not found")?;
        renderer.parse_svg(&bytes)?;
        return Ok(Artwork::Svg {
            bytes: Arc::from(bytes.as_ref()),
            embedded: true,
        });
    }
    let mut url = url::Url::parse(uri)?;
    url.set_fragment(None);
    let mut request = Request::get(url.as_str()).body(AsyncBody::default())?;
    request
        .extensions_mut()
        .insert(RequestTimeout(ICON_TIMEOUT));
    request.extensions_mut().insert(RedirectPolicy::FollowAll);
    let mut response = client.send(request).await?;
    if !response.status().is_success() {
        bail!("agent icon HTTP status {}", response.status());
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    response
        .body_mut()
        .take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_ICON_BYTES {
        bail!("agent icon exceeds {MAX_ICON_BYTES} bytes");
    }
    let path_mime = mime_guess::from_path(url.path()).first_or_octet_stream();
    let format = ImageFormat::from_mime_type(&mime)
        .or_else(|| ImageFormat::from_mime_type(path_mime.essence_str()));
    if matches!(format, Some(ImageFormat::Svg)) || format.is_none() {
        renderer.parse_svg(&bytes)?;
        Ok(Artwork::Svg {
            bytes: Arc::from(bytes),
            embedded: false,
        })
    } else {
        let format = format.context("agent icon image format unavailable")?;
        Ok(Artwork::Bitmap(
            Image::from_bytes(format, bytes).to_image_data(renderer)?,
        ))
    }
}
