//! GUI resolution of shared appearance preferences.
use super::source;
use anyhow::Result;
pub(super) use basr::config::appearance::positive;
pub(crate) use basr::config::appearance::{
    Appearance as Preferences, Background, ComposerEnterBehavior, Language, ThemeMode, ThemeSource,
    ValidationError,
};
use gpui_kit::WindowBackgroundAppearance;

#[cfg(test)]
mod tests;

pub(super) const DEFAULT_LINE_HEIGHT: f32 = 1.6;
const DEFAULT_UI_SIZE: f32 = 15.;
const CONTENT_FONT_SCALE: f32 = 0.875;
const DEFAULT_CONTENT_OPACITY: f32 = 0.94;

pub(super) struct Effective {
    pub language: Language,
    pub desktop: source::Resolved,
    pub content_font_size: f32,
    pub content_line_height: f32,
    pub content_opacity: f32,
    pub composer_enter_behavior: ComposerEnterBehavior,
}

/// # Errors
/// Validates explicit preferences and all effective font dimensions.
pub(super) fn resolve(
    preferences: &Preferences,
    mut desktop: source::Resolved,
) -> Result<Effective> {
    preferences.validate()?;
    let config = &mut desktop.config;
    if let Some(family) = &preferences.ui_font_family {
        config.font_family = Some(family.clone().into());
    }
    if let Some(family) = &preferences.mono_font_family {
        config.mono_font_family = Some(family.clone().into());
    }
    if let Some(fallbacks) = &preferences.font_fallbacks {
        desktop.font_fallbacks = fallbacks.clone();
    }
    let ui_size = preferences
        .ui_font_size
        .or(config.font_size)
        .unwrap_or(DEFAULT_UI_SIZE);
    positive("ui_font_size", ui_size)?;
    config.font_size = Some(ui_size);
    let content_font_size = preferences
        .content_font_size
        .or(desktop.content_font_size)
        .unwrap_or_else(|| (ui_size * CONTENT_FONT_SCALE).round().max(1.));
    positive("content_font_size", content_font_size)?;
    desktop.background = match preferences.background {
        Background::Auto => desktop.background,
        Background::Opaque => WindowBackgroundAppearance::Opaque,
        Background::Transparent => WindowBackgroundAppearance::Transparent,
        Background::Blurred => WindowBackgroundAppearance::Blurred,
    };
    Ok(Effective {
        language: preferences.language,
        desktop,
        content_font_size,
        content_line_height: preferences
            .content_line_height
            .unwrap_or(DEFAULT_LINE_HEIGHT),
        content_opacity: preferences
            .content_opacity
            .unwrap_or(DEFAULT_CONTENT_OPACITY),
        composer_enter_behavior: preferences.composer_enter_behavior,
    })
}
