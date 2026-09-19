use anyhow::{Context as _, Result};
use gpui_kit::{WindowBackgroundAppearance, component::ThemeConfig};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct Sources {
    zed_theme: PathBuf,
    zed_settings: PathBuf,
    dms_colors: PathBuf,
    dms_settings: PathBuf,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct Files {
    pub zed_theme: Option<String>,
    pub zed_settings: Option<String>,
    pub dms_colors: Option<String>,
    pub dms_settings: Option<String>,
}

pub(super) struct Resolved {
    pub config: ThemeConfig,
    pub background: WindowBackgroundAppearance,
    pub name: String,
    pub content_font_size: Option<f32>,
    pub font_fallbacks: Vec<String>,
}

impl Sources {
    pub fn discover(theme_file: Option<PathBuf>) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache"));
        Self {
            zed_theme: theme_file.unwrap_or_else(|| config.join("zed/themes/dank-zed-theme.json")),
            zed_settings: config.join("zed/settings.json"),
            dms_colors: cache.join("DankMaterialShell/dms-colors.json"),
            dms_settings: config.join("DankMaterialShell/settings.json"),
        }
    }
    pub fn read(&self) -> Files {
        Files {
            zed_theme: read(&self.zed_theme),
            zed_settings: read(&self.zed_settings),
            dms_colors: read(&self.dms_colors),
            dms_settings: read(&self.dms_settings),
        }
    }
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// # Errors
/// Rejects partially written or invalid theme JSON so live reload retains the last palette.
pub(super) fn resolve(files: &Files, dark: bool) -> Result<Resolved> {
    let settings = parse(files.zed_settings.as_deref())?;
    let desktop = parse(files.dms_settings.as_deref())?;
    let mut result = if let Some(theme) = files.zed_theme.as_deref() {
        from_zed(&parse(Some(theme))?, &settings, dark)?
    } else if let Some(palette) = files.dms_colors.as_deref() {
        from_palette(&parse(Some(palette))?, dark)?
    } else {
        fallback(dark)
    };
    result.config.font_family = settings["ui_font_family"]
        .as_str()
        .or_else(|| desktop["fontFamily"].as_str())
        .map(Into::into);
    result.config.mono_font_family = settings["buffer_font_family"]
        .as_str()
        .or_else(|| desktop["monoFontFamily"].as_str())
        .map(Into::into);
    result.config.font_size = Some(settings["ui_font_size"].as_f64().unwrap_or(15.) as f32);
    result.content_font_size = settings["agent_ui_font_size"]
        .as_f64()
        .map(|size| size as f32)
        .filter(|size| size.is_finite() && *size > 0.);
    result.font_fallbacks = settings["ui_font_fallbacks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    Ok(result)
}

/// # Errors
/// Reports invalid JSONC, including malformed intermediate writes.
fn parse(text: Option<&str>) -> Result<Value> {
    text.map(serde_json_lenient::from_str)
        .transpose()
        .context("invalid desktop theme JSON")
        .map(|value| value.unwrap_or(Value::Null))
}

/// # Errors
/// Reports a missing variant or invalid Zed theme schema.
fn from_zed(family: &Value, settings: &Value, dark: bool) -> Result<Resolved> {
    let selection = &settings["theme"];
    let mode = match selection["mode"].as_str() {
        Some("light") => "light",
        Some("dark") => "dark",
        _ => {
            if dark {
                "dark"
            } else {
                "light"
            }
        }
    };
    let requested = selection.as_str().or_else(|| selection[mode].as_str());
    let themes = family["themes"]
        .as_array()
        .context("Zed theme family has no variants")?;
    let theme = requested
        .and_then(|name| themes.iter().find(|theme| theme["name"] == name))
        .or_else(|| {
            themes.iter().find(|theme| {
                theme["appearance"] == mode
                    && theme["style"]["background.appearance"]
                        .as_str()
                        .is_some_and(|value| value != "opaque")
            })
        })
        .or_else(|| themes.iter().find(|theme| theme["appearance"] == mode))
        .context("Zed theme has no matching light/dark variant")?;
    let mut style = theme["style"].clone();
    if let (Some(style), Some(overrides)) = (
        style.as_object_mut(),
        settings["theme_overrides"].as_object(),
    ) {
        style.extend(overrides.clone());
    }
    let mut colors = serde_json::Map::new();
    for (target, source) in [
        ("background", "editor.background"),
        ("foreground", "text"),
        ("sidebar.background", "panel.background"),
        ("sidebar.foreground", "text"),
        ("sidebar.accent.background", "ghost_element.selected"),
        ("border", "border"),
        ("input.border", "border"),
        ("accent.background", "element.hover"),
        ("accent.foreground", "text"),
        ("muted.background", "element.background"),
        ("muted.foreground", "text.muted"),
        ("secondary.background", "element.selected"),
        ("secondary.foreground", "text"),
        ("primary.background", "text.accent"),
        ("primary.foreground", "editor.background"),
        ("ring", "border.focused"),
        ("caret", "text.accent"),
        ("link.foreground", "text.accent"),
        ("popover.background", "elevated_surface.background"),
        ("popover.foreground", "text"),
        ("scrollbar.thumb.background", "scrollbar.thumb.background"),
        (
            "scrollbar.thumb.hover.background",
            "scrollbar.thumb.hover_background",
        ),
        ("danger.background", "error"),
        ("success.background", "success"),
        ("warning.background", "warning"),
    ] {
        if style[source].is_string() {
            colors.insert(target.into(), style[source].clone());
        }
    }
    colors.insert("scrollbar.background".into(), json!("#00000000"));
    // Button text must stay opaque even when the editor surface is translucent.
    if let Some(Value::String(color)) = colors.get_mut("primary.foreground")
        && color.len() == 9
        && color.starts_with('#')
    {
        color.truncate(7);
    }
    let background = match style["background.appearance"].as_str() {
        Some("blurred") => WindowBackgroundAppearance::Blurred,
        Some("transparent") => WindowBackgroundAppearance::Transparent,
        _ => WindowBackgroundAppearance::Opaque,
    };
    let name = theme["name"].as_str().unwrap_or("DMS").to_owned();
    // Zed allows themes without syntax colors; GPUI's highlight schema requires the map.
    if style["syntax"].is_null() {
        style["syntax"] = json!({});
    }
    let config = serde_json::from_value(json!({
        "name": name, "mode": theme["appearance"], "radius": 6, "colors": colors, "highlight": style,
    })).context("cannot map the Zed theme to GPUI")?;
    Ok(Resolved {
        config,
        background,
        name,
        content_font_size: None,
        font_fallbacks: Vec::new(),
    })
}

/// # Errors
/// Reports missing DMS palette data or invalid colors.
fn from_palette(value: &Value, dark: bool) -> Result<Resolved> {
    let mode = if dark { "dark" } else { "light" };
    let palette = &value["colors"][mode];
    anyhow::ensure!(
        palette["surface"].is_string(),
        "DMS palette has no surface color"
    );
    let mut colors = serde_json::Map::new();
    for (target, source) in [
        ("background", "surface"),
        ("sidebar.background", "surface_container_lowest"),
        ("foreground", "on_surface"),
        ("muted.foreground", "on_surface_variant"),
        ("muted.background", "surface_container_high"),
        ("border", "outline_variant"),
        ("primary.background", "primary"),
        ("primary.foreground", "on_primary"),
        ("accent.background", "secondary_container"),
        ("accent.foreground", "on_secondary_container"),
        ("sidebar.accent.background", "secondary_container"),
        ("danger.background", "error"),
    ] {
        if palette[source].is_string() {
            colors.insert(target.into(), palette[source].clone());
        }
    }
    let config = serde_json::from_value(
        json!({ "name": "DMS", "mode": mode, "radius": 6, "colors": colors }),
    )?;
    Ok(Resolved {
        config,
        name: "DMS".into(),
        background: WindowBackgroundAppearance::Opaque,
        content_font_size: None,
        font_fallbacks: Vec::new(),
    })
}

pub(super) fn fallback(dark: bool) -> Resolved {
    let config = if dark {
        serde_json::from_str(include_str!("assets/dark.json")).expect("valid bundled theme")
    } else {
        serde_json::from_value(json!({
            "name": "BASR Light", "mode": "light", "font.size": 15, "radius": 6,
            "colors": {"background": "#fafafa", "foreground": "#25262b", "sidebar.background": "#f1f2f4",
                "primary.background": "#3864a5", "primary.foreground": "#ffffff"}
        })).expect("valid bundled light theme")
    };
    Resolved {
        config,
        name: "System".into(),
        background: WindowBackgroundAppearance::Opaque,
        content_font_size: None,
        font_fallbacks: Vec::new(),
    }
}
