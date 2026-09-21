//! BASR-owned policy, separate from desktop format adapters and runtime font lookup.

use super::source;
use anyhow::{Context as _, Result};
use gpui_kit::WindowBackgroundAppearance;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

pub(super) const DEFAULT_LINE_HEIGHT: f32 = 1.6;
const DEFAULT_UI_SIZE: f32 = 15.;
const CONTENT_FONT_SCALE: f32 = 0.875;
const DEFAULT_CONTENT_OPACITY: f32 = 0.94;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Preferences {
    pub ui_font_family: Option<String>,
    pub mono_font_family: Option<String>,
    pub font_fallbacks: Option<Vec<String>>,
    pub ui_font_size: Option<f32>,
    pub content_font_size: Option<f32>,
    pub content_line_height: Option<f32>,
    pub content_opacity: Option<f32>,
    pub background: Background,
    pub theme_source: ThemeSource,
    pub theme_mode: ThemeMode,
    pub theme_file: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Background {
    Auto,
    Opaque,
    Transparent,
    #[default]
    Blurred,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThemeSource {
    #[default]
    Desktop,
    /// Do not import desktop colors or fonts.
    Builtin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThemeMode {
    #[default]
    Desktop,
    System,
    Light,
    Dark,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ValidationError {
    #[error("{0} must be finite and greater than zero")]
    Positive(&'static str),
    #[error("content_opacity must be finite and between 0 and 1")]
    Opacity,
    #[error("{0} must not contain an empty font name")]
    Font(&'static str),
    #[error("theme_file must not be empty")]
    ThemeFile,
}

impl Preferences {
    /// # Errors
    /// Rejects invalid sizes, opacity, font names, and empty theme paths.
    pub(crate) fn validate(&self) -> std::result::Result<(), ValidationError> {
        for (name, value) in [
            ("ui_font_size", self.ui_font_size),
            ("content_font_size", self.content_font_size),
            ("content_line_height", self.content_line_height),
        ] {
            if let Some(value) = value {
                positive(name, value)?;
            }
        }
        if self
            .content_opacity
            .is_some_and(|v| !v.is_finite() || !(0. ..=1.).contains(&v))
        {
            return Err(ValidationError::Opacity);
        }
        for (name, value) in [
            ("ui_font_family", self.ui_font_family.as_deref()),
            ("mono_font_family", self.mono_font_family.as_deref()),
        ] {
            if value.is_some_and(|v| v.trim().is_empty()) {
                return Err(ValidationError::Font(name));
            }
        }
        if self
            .font_fallbacks
            .as_ref()
            .is_some_and(|v| v.iter().any(|v| v.trim().is_empty()))
        {
            return Err(ValidationError::Font("font_fallbacks"));
        }
        if self
            .theme_file
            .as_ref()
            .is_some_and(|v| v.as_os_str().is_empty())
        {
            return Err(ValidationError::ThemeFile);
        }
        Ok(())
    }

    /// # Errors
    /// Reports invalid JSON or invalid preference values.
    pub(crate) fn from_json(text: &str) -> Result<Self> {
        let preferences: Self =
            serde_json::from_str(text).context("invalid BASR appearance JSON")?;
        preferences.validate()?;
        Ok(preferences)
    }
}

/// # Errors
/// Rejects zero, negative, NaN, and infinite sizes, including desktop imports.
pub(super) fn positive(name: &'static str, value: f32) -> Result<(), ValidationError> {
    if value.is_finite() && value > 0. {
        Ok(())
    } else {
        Err(ValidationError::Positive(name))
    }
}

pub(super) struct Effective {
    pub desktop: source::Resolved,
    pub content_font_size: f32,
    pub content_line_height: f32,
    pub content_opacity: f32,
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
        desktop,
        content_font_size,
        content_line_height: preferences
            .content_line_height
            .unwrap_or(DEFAULT_LINE_HEIGHT),
        content_opacity: preferences
            .content_opacity
            .unwrap_or(DEFAULT_CONTENT_OPACITY),
    })
}
