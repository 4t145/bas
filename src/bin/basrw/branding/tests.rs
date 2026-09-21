use std::{borrow::Cow, sync::Arc};

use gpui_kit::{
    AssetSource, DevicePixels, SvgRenderer, SvgSize, TestApp, component::ThemeMode, size,
};

use super::*;
use crate::assets::Assets;

#[test]
fn assets_use_the_confirmed_regular_v4_outlines() {
    assert_eq!(OUTLINE, "branding/ferris-outline.svg");
    assert_eq!(OUTLINE_LIGHT, "branding/ferris-outline-light.svg");
    let registered = Assets.list("branding/").expect("branding paths");
    for path in [OUTLINE, OUTLINE_LIGHT] {
        let bytes = Assets
            .load(path)
            .expect("outline loads")
            .expect("outline exists");
        let svg = std::str::from_utf8(&bytes).expect("SVG text");
        assert!(
            svg.contains("width=\"512\" height=\"512\""),
            "regular source dimensions"
        );
        assert_eq!(
            svg.matches("M-23-112Q0-122").count(),
            8,
            "three-part connectors retained"
        );
        assert!(
            svg.contains("M347 591C373 613 416 615 443 591"),
            "curved closed eyes retained"
        );
    }
    assert!(
        !registered
            .iter()
            .any(|path| { path.contains("outline-32") || path.contains("outline-light-32") }),
        "simplified/modified outlines are not shipped"
    );
}

#[test]
fn assets_load_list_and_icon_fallback() {
    let paths = Assets.list("branding/").expect("branding assets list");
    assert_eq!(paths.len(), 4);
    for path in &paths {
        assert!(matches!(
            Assets.load(path).expect("branding asset loads"),
            Some(Cow::Borrowed(_))
        ));
    }
    assert_eq!(Assets.list(OUTLINE).expect("exact prefix"), [OUTLINE]);
    assert!(Assets.list("missing/").expect("unknown prefix").is_empty());

    // Both the application extras and the component bundle must remain available.
    assert!(
        Assets
            .load("icons/refresh-cw.svg")
            .expect("extra icon")
            .is_some()
    );
    for path in gpui_kit::assets::Assets
        .list("")
        .expect("upstream icon list")
    {
        assert_eq!(
            Assets.load(&path).expect("fallback icon"),
            gpui_kit::assets::Assets.load(&path).expect("upstream icon"),
        );
    }
    let all = Assets.list("").expect("all assets");
    assert!(all.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(paths.iter().all(|path| all.contains(path)));
    assert!(Assets.load("").expect("empty path").is_none());
    assert_eq!(
        Assets.load("missing.svg").is_err(),
        gpui_kit::assets::Assets.load("missing.svg").is_err(),
    );
}

#[test]
fn assets_are_self_contained_vectors() {
    for path in Assets.list("branding/").expect("branding assets list") {
        let bytes = Assets.load(&path).expect("load SVG").expect("embedded SVG");
        let svg = std::str::from_utf8(&bytes).expect("UTF-8 SVG");
        let lower = svg.to_ascii_lowercase();
        // These supplied files need neither links, fonts, scripts nor CSS imports.
        for forbidden in [
            "href",
            "<image",
            "<text",
            "<script",
            "<style",
            "<foreignobject",
            "<!doctype",
            "<!entity",
            "<?",
            "@import",
            "font-family",
            "data:",
        ] {
            assert!(!lower.contains(forbidden), "{path}: {forbidden}");
        }
        for reference in svg.split("url(").skip(1) {
            let id = reference
                .split_once(')')
                .expect("closed SVG paint reference")
                .0
                .strip_prefix('#')
                .expect("paint references must be document-local");
            assert!(svg.contains(&format!("id=\"{id}\"")), "{path}: {id}");
        }
        assert_eq!(svg.matches("data-connector=").count(), 8, "{path}");
    }
}

#[test]
fn assets_decode_with_gpui_svg_renderer_preserving_color_and_transparency() {
    // This is the same parser/rasterizer used by GPUI's image asset loader.
    let renderer = SvgRenderer::new(Arc::new(Assets));
    for path in Assets.list("branding/").expect("branding assets list") {
        let bytes = Assets.load(&path).expect("load SVG").expect("embedded SVG");
        let parsed = renderer
            .parse_svg(&bytes)
            .expect("GPUI parses original SVG");
        for pixels in [28, 32, 64] {
            let dimensions = size(DevicePixels(pixels), DevicePixels(pixels));
            let image = renderer
                .render_parsed(&parsed, SvgSize::ExactSize(dimensions))
                .expect("GPUI rasterizes original SVG");
            assert_eq!(image.size(0), dimensions);
            assert_eq!(image.frame_count(), 1);
            let data = image.as_bytes(0).expect("BGRA pixels");
            assert!(
                data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0),
                "{path}"
            );
            assert_eq!(data[3], 0, "transparent outer corner: {path}");
            if path.as_ref() == OUTLINE || path.as_ref() == OUTLINE_LIGHT {
                let center = ((pixels / 2 * pixels + pixels / 2) * 4) as usize;
                assert_eq!(data[center + 3], 0, "transparent shell interior: {path}");
                let expected: [u8; 3] = if path.as_ref() == OUTLINE_LIGHT {
                    [0xdb, 0xed, 0xf6]
                } else {
                    [0x27, 0x27, 0x26]
                };
                // GPUI unpremultiplies the rasterizer's 8-bit pixels. Account for
                // that quantization at thin edges instead of requiring opaque ink.
                assert!(
                    data.as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|pixel| pixel[3] > 0)
                        .all(
                            |pixel| pixel[..3].iter().zip(expected).all(|(actual, color)| {
                                let alpha = u16::from(pixel[3]);
                                u16::from(*actual).abs_diff(u16::from(color)) * alpha <= 255 + alpha
                            })
                        ),
                    "original outline ink color: {path}",
                );
            } else {
                assert!(
                    data.as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[3] > 200 && pixel[2] > 180 && pixel[0] < 100),
                    "dimensional orange is not a monochrome mask: {path}",
                );
            }
        }
    }
}

#[test]
fn assets_logo_follows_theme_and_can_be_resized() {
    let mut app = TestApp::new();
    app.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        assert_eq!(logo_path(cx), OUTLINE);
        let _small = logo(cx).size(px(28.));
        Theme::change(ThemeMode::Dark, None, cx);
        assert_eq!(logo_path(cx), OUTLINE_LIGHT);
        let _default = logo(cx);
    });
}
