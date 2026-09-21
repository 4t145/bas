//! Resolve BASR preferences over desktop adapters, then apply the effective appearance.

mod loader;
pub(crate) mod preferences;
mod source;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

use gpui_kit::{
    App, Global, Pixels, Task, Window, WindowBackgroundAppearance,
    component::{Theme, ThemeConfig, ThemeMode},
    px,
};
use loader::Loader;
use preferences::{Effective, Preferences};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

pub(crate) use loader::Document;

pub(crate) struct Editor {
    loader: Loader,
    generation: Rc<Cell<u64>>,
}

impl Editor {
    pub(crate) fn current(cx: &App) -> Self {
        let config = cx.global::<WatchConfig>();
        Self {
            loader: config.loader.clone(),
            generation: config.generation.clone(),
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.loader.appearance_file
    }

    pub(crate) fn theme_override(&self) -> Option<&std::path::Path> {
        self.loader.theme_file.as_deref()
    }

    /// # Errors
    /// Returns configuration read/parse errors, preserving the original file.
    pub(crate) fn load(&self) -> anyhow::Result<Document> {
        self.loader.document()
    }

    /// # Errors
    /// Validation, conflicts or IO failures leave the current appearance intact.
    pub(crate) fn save(
        &self,
        document: &mut Document,
        candidate: Preferences,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let dark = ThemeMode::from(window.appearance()).is_dark();
        let effective = self.loader.save(document, candidate, dark)?;
        self.generation.set(self.generation.get().wrapping_add(1));
        apply(effective, Some(window), cx);
        Ok(())
    }

    #[cfg(all(test, feature = "gui-tests"))]
    pub(crate) fn at(path: PathBuf) -> Self {
        let mut loader = Loader::new(Some(path), None);
        loader.test_imports = Some(source::Files::default());
        Self {
            loader,
            generation: Rc::default(),
        }
    }
}

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(super) fn content_font_size(cx: &App) -> Pixels {
    px(cx.global::<Appearance>().content_font_size)
}

pub(super) fn content_line_height(cx: &App) -> f32 {
    cx.global::<Appearance>().content_line_height
}

pub(super) fn content_background(cx: &App) -> gpui_kit::Hsla {
    let mut color = Theme::global(cx).background;
    color.a = cx.global::<Appearance>().content_opacity;
    color
}

#[derive(Clone)]
struct WatchConfig {
    loader: Loader,
    generation: Rc<Cell<u64>>,
}
impl Global for WatchConfig {}

struct WatchTask {
    _task: Task<()>,
}
impl Global for WatchTask {}

#[derive(Clone)]
pub(super) struct Appearance {
    pub name: String,
    pub background: WindowBackgroundAppearance,
    pub content_font_size: f32,
    pub content_line_height: f32,
    pub content_opacity: f32,
    pub font_fallbacks: gpui_kit::FontFallbacks,
}
impl Global for Appearance {}

#[cfg(all(test, feature = "gui-tests"))]
pub(super) fn init_default(cx: &mut App) {
    let effective = preferences::resolve(&Preferences::default(), source::fallback(true))
        .expect("built-in appearance is valid");
    apply(effective, None, cx);
}

pub(super) fn init(theme_file: Option<PathBuf>, appearance_file: Option<PathBuf>, cx: &mut App) {
    let loader = Loader::new(appearance_file, theme_file);
    let dark = ThemeMode::from(cx.window_appearance()).is_dark();
    let effective = loader.read().and_then(|snapshot| snapshot.resolve(dark))
        .unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "cannot load appearance; using built-in defaults");
            preferences::resolve(&Preferences::default(), source::fallback(dark))
                .expect("built-in appearance is valid")
        });
    apply(effective, None, cx);
    cx.set_global(WatchConfig {
        loader,
        generation: Rc::default(),
    });
}

pub(super) fn watch(cx: &mut App) {
    if cx.has_global::<WatchTask>() {
        return;
    }
    let config = cx.global::<WatchConfig>().clone();
    let sources = config.loader;
    let task = cx.spawn(async move |cx| {
        let mut reload = loader::Reload::default();
        let mut previous_error = None;
        loop {
            let generation = config.generation.get();
            let source = sources.clone();
            let files = cx
                .background_executor()
                .spawn(async move { source.read() })
                .await;
            cx.update(|cx| {
                    // Saves can occur during either the read or the polling interval.
                    reload.invalidate_on_save(config.generation.get());
                    if generation != config.generation.get() {
                        return;
                    }
                    let dark = ThemeMode::from(cx.window_appearance()).is_dark();
                    match files.and_then(|snapshot| reload.update(snapshot, dark)) {
                        Ok(effective) => {
                            previous_error = None;
                            if let Some(effective) = effective {
                                apply(effective, None, cx);
                            }
                        }
                        Err(error) => {
                            let error = format!("{error:#}");
                            if previous_error.as_ref() != Some(&error) {
                                tracing::warn!(%error, "cannot reload appearance; keeping the last valid configuration");
                                previous_error = Some(error);
                            }
                        }
                    }
                });
            cx.background_executor().timer(REFRESH_INTERVAL).await;
        }
    });
    cx.set_global(WatchTask { _task: task });
}

fn apply(effective: Effective, window: Option<&mut Window>, cx: &mut App) {
    let mut resolved = effective.desktop;
    let mut config: ThemeConfig = resolved.config;
    let fonts = cx.text_system().all_font_names();
    let preferred = config.font_family.take();
    config.font_family = preferred
        .filter(|family| fonts.iter().any(|font| font == family.as_ref()))
        .or_else(|| {
            ["Inter", "Inter Variable", "Noto Sans", "Noto Sans CJK SC"]
                .into_iter()
                .find(|name| fonts.iter().any(|font| font == name))
                .map(Into::into)
        });
    if config
        .mono_font_family
        .as_ref()
        .is_some_and(|family| !fonts.iter().any(|font| font == family.as_ref()))
    {
        config.mono_font_family = None;
    }
    resolved
        .font_fallbacks
        .retain(|family| fonts.contains(family));
    // Runtime font availability/platform fallback, not persisted user preferences.
    // Prefer static CJK faces to avoid thin default axes in Linux font backends.
    for family in [
        "Sarasa UI SC",
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "PingFang SC",
        "Microsoft YaHei",
    ] {
        if fonts.iter().any(|font| font == family) {
            if !resolved.font_fallbacks.iter().any(|name| name == family) {
                resolved.font_fallbacks.push(family.to_owned());
            }
            break;
        }
    }
    tracing::debug!(family = ?config.font_family, fallbacks = ?resolved.font_fallbacks, "resolved UI font families");
    let mode = config.mode;
    let config = Rc::new(config);
    let theme = Theme::global_mut(cx);
    if mode.is_dark() {
        theme.dark_theme = config;
    } else {
        theme.light_theme = config;
    }
    cx.set_global(Appearance {
        name: resolved.name,
        background: resolved.background,
        content_font_size: effective.content_font_size,
        content_line_height: effective.content_line_height,
        content_opacity: effective.content_opacity,
        font_fallbacks: gpui_kit::FontFallbacks::from_fonts(resolved.font_fallbacks),
    });
    if let Some(window) = window {
        window.set_background_appearance(resolved.background);
        Theme::change(mode, Some(window), cx);
    } else {
        Theme::change(mode, None, cx);
    }
    cx.refresh_windows();
    // The saving window may currently be borrowed; apply platform surfaces after
    // that callback, and read the latest value if several edits were coalesced.
    cx.defer(|cx| {
        let background = cx.global::<Appearance>().background;
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| {
                window.set_background_appearance(background);
            });
        }
    });
}

pub(super) fn font_style(cx: &App) -> gpui_kit::StyleRefinement {
    let mut style = gpui_kit::StyleRefinement::default();
    style.text.font_fallbacks = cx
        .try_global::<Appearance>()
        .map(|appearance| appearance.font_fallbacks.clone());
    style
}
