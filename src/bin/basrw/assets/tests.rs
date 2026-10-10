use std::sync::Arc;

use gpui_kit::{DevicePixels, SvgRenderer, SvgSize, assets::IconName, size};

use super::*;

#[test]
fn registry_icons_are_embedded_and_render_visible_svg_pixels() {
    let paths = Assets.list("icons/").expect("application icon paths");
    let renderer = SvgRenderer::new(Arc::new(Assets));
    for icon in [
        IconName::Download,
        IconName::Scale,
        IconName::Bot,
        IconName::Github,
        IconName::ExternalLink,
        IconName::Search,
        IconName::RefreshCw,
        IconName::CornerLeftUp,
    ] {
        let path = icon.path();
        assert!(paths.contains(&path), "registered icon: {path}");
        let bytes = Assets
            .load(&path)
            .expect("load icon")
            .expect("embedded icon");
        let parsed = renderer.parse_svg(&bytes).expect("GPUI parses icon SVG");
        let image = renderer
            .render_parsed(
                &parsed,
                SvgSize::ExactSize(size(DevicePixels(24), DevicePixels(24))),
            )
            .expect("GPUI rasterizes icon SVG");
        assert!(
            image
                .as_bytes(0)
                .expect("icon pixels")
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0),
            "visible SVG artwork: {path}",
        );
    }
}
