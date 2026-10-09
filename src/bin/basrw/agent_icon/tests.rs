use std::sync::{Arc, Mutex};

use gpui_kit::{
    Context, Render,
    component::{Theme, ThemeMode},
    http_client::{FakeHttpClient, Response},
    size,
    test::TestWindowExt,
};

use super::*;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path fill="black" d="M2 2H8V8H2z"/></svg>"#;

struct Fixture;

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .gap_2()
            .child(
                AgentIcon::new("codex-thread", "https://example.com/logo.svg?v=1#mark")
                    .size(px(16.)),
            )
            .child(
                AgentIcon::new("codex-registry", "https://example.com/logo.svg?v=1#mark")
                    .size(px(24.)),
            )
            .child(AgentIcon::new("raster", "https://example.com/color.png").size(px(16.)))
            .child(AgentIcon::new(
                "malformed",
                "https://example.com/malformed.svg",
            ))
            .child(AgentIcon::new("missing", "https://example.com/missing.svg"))
            .child(AgentIcon::new(
                "oversized",
                "https://example.com/oversized.svg",
            ))
            .child(AgentIcon::new("stalled", "https://example.com/stalled.svg"))
            .child(AgentIcon::new("custom", ""))
    }
}

fn png() -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGM44cb/HwAFFAId9TTI2AAAAABJRU5ErkJggg==")
        .expect("colored PNG fixture")
}

#[gpui_kit::test]
fn real_svg_and_png_are_cached_across_rows_sizes_and_themes_with_failed_fallbacks(
    cx: &mut gpui_kit::TestAppContext,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_http_client(FakeHttpClient::create(move |request| {
            recorded
                .lock()
                .expect("requests")
                .push(request.uri().to_string());
            assert_eq!(
                request.extensions().get::<RequestTimeout>().map(|t| t.0),
                Some(ICON_TIMEOUT)
            );
            let path = request.uri().path().to_owned();
            async move {
                if path == "/stalled.svg" {
                    return futures::future::pending().await;
                }
                let (status, mime, bytes) = match path.as_str() {
                    "/color.png" => (200, "image/png", png()),
                    "/malformed.svg" => (200, "image/svg+xml", b"broken SVG".to_vec()),
                    "/missing.svg" => (404, "text/plain", b"missing".to_vec()),
                    "/oversized.svg" => (
                        200,
                        "image/svg+xml",
                        vec![b' '; MAX_ICON_BYTES as usize + 1],
                    ),
                    _ => (200, "image/svg+xml", SVG.as_bytes().to_vec()),
                };
                Ok(Response::builder()
                    .status(status)
                    .header("Content-Type", mime)
                    .body(bytes.into())
                    .expect("fixture response"))
            }
        }));
    });
    let window = cx.open_window(size(px(700.), px(150.)), |window, cx| {
        let view = cx.new(|_| Fixture);
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("initial frame");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("codex-thread-svg").visible());
        assert!(window.find("codex-registry-svg").visible());
        assert_eq!(
            window.find("codex-thread-svg").bounds().size,
            size(px(16.), px(16.))
        );
        assert_eq!(
            window.find("codex-registry-svg").bounds().size,
            size(px(24.), px(24.))
        );
        assert!(window.find("raster-image").visible());
        for id in ["malformed", "missing", "oversized", "stalled", "custom"] {
            assert!(window.find(format!("{id}-fallback")).visible());
        }
        let cached = cx.global::<Cache>();
        let Load::Ready(Artwork::Svg {
            bytes,
            embedded: false,
        }) = &cached.0["https://example.com/logo.svg?v=1#mark"]
        else {
            panic!("original SVG is cached as a themed vector");
        };
        assert_eq!(
            bytes.as_ref(),
            SVG.as_bytes(),
            "the original artwork bytes are unchanged"
        );
        let Load::Ready(Artwork::Bitmap(image)) = &cached.0["https://example.com/color.png"] else {
            panic!("PNG is cached as a full color bitmap");
        };
        assert_eq!(
            image.as_bytes(0).expect("PNG pixels"),
            &[15, 70, 200, 255],
            "original PNG color is preserved"
        );
        let renderer = cx.svg_renderer();
        let parsed = renderer.parse_svg(bytes).expect("original SVG");
        let image = renderer
            .render_parsed(
                &parsed,
                gpui_kit::SvgSize::ExactSize(size(
                    gpui_kit::DevicePixels(16),
                    gpui_kit::DevicePixels(16),
                )),
            )
            .expect("actual SVG rasterization");
        let pixels = image.as_bytes(0).expect("SVG pixels");
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        assert_eq!(
            pixels[3], 0,
            "SVG remains transparent outside the actual logo"
        );
        Theme::change(ThemeMode::Light, Some(window), cx);
        window.render_frame(cx);
        assert!(window.find("codex-thread-svg").visible());
        assert!(window.find("codex-registry-svg").visible());
    })
    .expect("loaded icons and theme adaptation");
    cx.run_until_parked();
    cx.executor().advance_clock(ICON_TIMEOUT);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("stalled-fallback").visible());
        assert!(matches!(
            cx.global::<Cache>().0["https://example.com/stalled.svg"],
            Load::Failed
        ));
    })
    .expect("timeout fallback");
    assert_eq!(
        requests.lock().expect("requests").len(),
        6,
        "one request per URI including failed and timed out requests"
    );
    assert_eq!(
        requests
            .lock()
            .expect("requests")
            .iter()
            .filter(|uri| uri.as_str() == "https://example.com/logo.svg?v=1")
            .count(),
        1,
        "fragment never goes to HTTP and thread/registry share one request"
    );
}

#[test]
fn embedded_artwork_is_loaded_from_the_real_application_bundle() {
    let assets: Arc<dyn AssetSource> = Arc::new(crate::assets::Assets);
    let renderer = SvgRenderer::new(assets.clone());
    // TestAppContext owns an empty asset source; validate the application bundle directly.
    let artwork = futures::executor::block_on(load(
        crate::assets::OUTLINE,
        FakeHttpClient::with_404_response(),
        renderer.clone(),
        assets.clone(),
    ))
    .expect("real embedded artwork");
    let Artwork::Svg { bytes, embedded } = artwork else {
        panic!("embedded artwork stays a vector");
    };
    assert!(embedded, "embedded artwork uses the registered asset path");
    assert_eq!(
        bytes.as_ref(),
        assets
            .load(crate::assets::OUTLINE)
            .expect("bundle asset")
            .expect("embedded asset")
            .as_ref(),
        "actual supplied artwork is retained",
    );
    let image = renderer
        .render_single_frame(&bytes, 1.)
        .expect("real bundle SVG rasterizes");
    assert!(
        image
            .as_bytes(0)
            .expect("pixels")
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0)
    );
}
