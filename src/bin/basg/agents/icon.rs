//! Real agent artwork shared by the registry and conversation rows.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{FutureExt as _, future::Either};
use gds_uim::el;
use gpui_kit::{
    App, Global, Hsla, IntoElement, ParentElement, Pixels, RenderImage, RenderOnce, SharedString,
    Styled, Task, TestSupportExt, Window,
    assets::IconName,
    component::{ActiveTheme as _, Icon},
    div, img,
    prelude::*,
    px, svg,
};

mod disk;
mod loading;
#[cfg(all(test, feature = "gui-tests"))]
mod tests;

const DEFAULT_SIZE: f32 = 24.;
const ICON_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone)]
enum Artwork {
    Svg { bytes: Arc<[u8]>, embedded: bool },
    Bitmap(Arc<RenderImage>),
}

#[derive(Default)]
struct Load {
    artwork: Option<Artwork>,
    task: Option<Task<()>>,
    refresh_at: Option<Instant>,
}

struct Cache {
    entries: HashMap<SharedString, Load>,
    disk: Option<disk::Cache>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            // Tests explicitly supply temporary caches, never the user's cache.
            disk: if cfg!(test) {
                None
            } else {
                disk::Cache::discover()
            },
        }
    }
}

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
                    el! { .svg { path: ~$(self.uri); } }
                } else {
                    el! { .svg { data: $&bytes; } }
                };
                (
                    "svg",
                    el! { logo { size: $self.size; text_color: color; } }.into_any_element(),
                )
            }
            Some(Artwork::Bitmap(image)) => (
                "image",
                el! { image.img { size: $self.size; } }.into_any_element(),
            ),
            None => (
                "fallback",
                el! {
                    IconName::Bot.Icon::new {
                        size: $self.size;
                        text_color: color;
                    }
                }
                .into_any_element(),
            ),
        };
        el! {
            .div {
                # ~$(self.id);
                test_support;
                size: $self.size;
                flex_shrink_0;
            } [
                .div {
                    # format!("{}-{kind}", self.id);
                    test_support;
                    size: $self.size;
                } [content],
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
    let key: SharedString = loading::key(uri).into();
    let now = cx.background_executor().now();
    let existing = cx.global::<Cache>().entries.get(&key);
    let artwork = existing.and_then(|load| load.artwork.clone());
    if existing.is_some_and(|load| {
        load.task.is_some() || load.refresh_at.is_none_or(|deadline| now < deadline)
    }) {
        return artwork;
    }
    let loader = loading::Loader {
        uri: key.to_string(),
        client: cx.http_client(),
        renderer: cx.svg_renderer(),
        assets: cx.asset_source().clone(),
        disk: cx.global::<Cache>().disk.clone(),
    };
    let executor = cx.background_executor().clone();
    let initial = executor.spawn(async move {
        let result = loader.cached();
        (loader, result)
    });
    let task_key = key.clone();
    let task = cx.spawn(async move |cx| {
        let (loader, initial) = initial.await;
        let initial = match initial {
            Ok(initial) => initial,
            Err(_) => {
                cx.update(|cx| finish(&task_key, None, Some(RETRY_AFTER), cx));
                return;
            }
        };
        if let Some(ready) = &initial {
            if ready.refresh_after != Some(Duration::ZERO) {
                cx.update(|cx| {
                    finish(
                        &task_key,
                        Some(ready.artwork.clone()),
                        ready.refresh_after,
                        cx,
                    )
                });
                return;
            }
            // Publish stale artwork without dropping the in-flight refresh task.
            cx.update(|cx| {
                cx.global_mut::<Cache>()
                    .entries
                    .get_mut(&task_key)
                    .expect("pending icon")
                    .artwork = Some(ready.artwork.clone());
                cx.refresh_windows();
            });
        }
        let timeout = executor.timer(ICON_TIMEOUT);
        let refresh = executor.spawn(async move {
            match futures::future::select(loader.fetch(initial).boxed(), timeout.boxed()).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => Err(anyhow::anyhow!("agent icon request timed out")),
            }
        });
        let result = refresh.await;
        cx.update(|cx| match result {
            Ok(ready) => finish(&task_key, Some(ready.artwork), ready.refresh_after, cx),
            Err(_) => {
                // Transport errors can contain signed URLs; do not log them.
                tracing::debug!("Agent icon refresh unavailable; retaining cached artwork");
                finish(&task_key, None, Some(RETRY_AFTER), cx);
            }
        });
    });
    cx.global_mut::<Cache>()
        .entries
        .entry(key)
        .or_default()
        .task = Some(task);
    artwork
}

fn finish(
    key: &SharedString,
    artwork: Option<Artwork>,
    refresh_after: Option<Duration>,
    cx: &mut App,
) {
    let deadline = refresh_after.map(|delay| cx.background_executor().now() + delay);
    let load = cx
        .global_mut::<Cache>()
        .entries
        .get_mut(key)
        .expect("pending icon");
    if artwork.is_some() {
        load.artwork = artwork;
    }
    load.refresh_at = deadline;
    load.task = None;
    cx.refresh_windows();
}
