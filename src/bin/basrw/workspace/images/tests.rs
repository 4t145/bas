use std::sync::{Arc, Mutex};

use gpui_kit::base::TextViewState;
use gpui_kit::{
    DevicePixels, TestAppContext, WindowHandle,
    http_client::{FakeHttpClient, Response},
    size,
    test::TestWindowExt,
};

use super::*;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;

fn png() -> Vec<u8> {
    include_bytes!("assets/sample.png").to_vec()
}

struct Block {
    id: &'static str,
    cache: Entity<Cache>,
    markdown: Option<Entity<TextViewState>>,
    image: Option<basr::acp::ImageContent>,
    probe: Option<Probe>,
}

#[derive(Clone)]
struct Probe {
    source: Resource,
    frames: Arc<Mutex<Vec<ImageLoad>>>,
}

type ImageLoad = Option<Result<Arc<RenderImage>, ImageCacheError>>;

impl Block {
    fn with_probe(mut self, source: &str) -> Self {
        self.probe = Some(Probe {
            source: Resource::Uri(source.to_owned().into()),
            frames: Arc::default(),
        });
        self
    }
}

struct Fixture {
    blocks: Vec<Block>,
    _changed: Vec<Subscription>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .children(self.blocks.iter().map(|block| {
                let content = if let Some(document) = &block.markdown {
                    wrap(
                        block.id,
                        &block.cache,
                        markdown::view(document, cx).into_any_element(),
                        cx,
                    )
                } else {
                    acp_image(
                        block.id,
                        block.image.as_ref().expect("ACP image"),
                        &block.cache,
                        false,
                        cx,
                    )
                };
                let cache = block.cache.clone();
                let probe = block.probe.clone();
                div()
                    .on_children_prepainted(move |_, window, cx| {
                        if let Some(probe) = &probe {
                            let result =
                                cache.update(cx, |cache, cx| cache.load(&probe.source, window, cx));
                            probe.frames.lock().expect("image probes").push(result);
                        }
                    })
                    .id(block.id)
                    .test_support()
                    .w(px(300.))
                    .flex_shrink_0()
                    .child(content)
            }))
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
    });
}

fn window(cx: &mut TestAppContext, blocks: Vec<Block>) -> WindowHandle<Root> {
    cx.open_window(size(px(500.), px(900.)), |window, cx| {
        let fixture = cx.new(|cx| {
            let mut changed: Vec<_> = blocks
                .iter()
                .map(|block| cx.observe(&block.cache, |_, _, cx| cx.notify()))
                .collect();
            changed.extend(
                blocks
                    .iter()
                    .filter_map(|block| block.markdown.as_ref())
                    .map(|state| cx.observe(state, |_, _, cx| cx.notify())),
            );
            Fixture {
                blocks,
                _changed: changed,
            }
        });
        Root::new(fixture, window, cx)
    })
}

fn markdown_block(
    id: &'static str,
    source: String,
    directory: Option<PathBuf>,
    cx: &mut App,
) -> Block {
    Block {
        id,
        cache: Cache::new(directory, cx),
        markdown: Some(cx.new(|cx| TextViewState::markdown(&source, cx))),
        image: None,
        probe: None,
    }
}

fn assert_png(cache: &Entity<Cache>, probe: &Probe, cx: &App) -> Arc<RenderImage> {
    let image = probe
        .frames
        .lock()
        .expect("image probes")
        .last()
        .expect("prepaint probe ran")
        .clone()
        .expect("completed image load")
        .expect("decoded image");
    assert_eq!(
        image.size(0),
        size(DevicePixels(WIDTH as i32), DevicePixels(HEIGHT as i32))
    );
    assert_eq!(
        &image.as_bytes(0).expect("pixels")[..4],
        &[15, 70, 200, 255],
        "PNG colors remain intact"
    );
    assert!(
        cache.read(cx).errors.is_empty(),
        "real image loads without fallback"
    );
    image
}

#[gpui_kit::test]
fn markdown_local_paths_with_spaces_file_uri_and_relative_directory_render_real_png(
    cx: &mut TestAppContext,
) {
    init(cx);
    let directory = tempfile::tempdir().expect("image directory");
    let path = directory
        .path()
        .join("Screenshot from 2026-10-10 14-22-58.png");
    std::fs::write(&path, png()).expect("local PNG");
    let absolute = path.to_string_lossy().into_owned();
    let file = url::Url::from_file_path(&path)
        .expect("file URI")
        .to_string();
    assert!(file.contains("%20"));
    let relative = path
        .file_name()
        .expect("filename")
        .to_string_lossy()
        .into_owned();
    let sources = [absolute, file, relative];
    let blocks = cx.update(|cx| {
        ["absolute", "file", "relative"]
            .into_iter()
            .zip(&sources)
            .map(|(id, source)| {
                markdown_block(
                    id,
                    format!("![截图](<{source}>)"),
                    Some(directory.path().to_owned()),
                    cx,
                )
                .with_probe(source)
            })
            .collect::<Vec<_>>()
    });
    let caches: Vec<_> = blocks.iter().map(|block| block.cache.clone()).collect();
    let probes: Vec<_> = blocks
        .iter()
        .filter_map(|block| block.probe.clone())
        .collect();
    let window = window(cx, blocks);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("start local loads");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for ((id, probe), cache) in ["absolute", "file", "relative"]
            .into_iter()
            .zip(&probes)
            .zip(&caches)
        {
            assert_png(cache, probe, cx);
            let bounds = window.find(id).bounds();
            assert!(
                bounds.size.height >= px(HEIGHT as f32),
                "Markdown reserves image height: {bounds:?}"
            );
            assert!(bounds.size.width <= px(300.));
        }
    })
    .expect("local Markdown images");
}

#[gpui_kit::test]
fn acp_and_markdown_data_images_decode_once_and_failed_images_have_visible_fallbacks(
    cx: &mut TestAppContext,
) {
    init(cx);
    let encoded = base64::engine::general_purpose::STANDARD.encode(png());
    let uri = format!("data:image/png;base64,{encoded}");
    let bad_uri = "data:image/png;base64,broken";
    let blocks = cx.update(|cx| {
        vec![
            Block {
                id: "acp",
                cache: Cache::new(None, cx),
                markdown: None,
                image: Some(basr::acp::ImageContent::new(encoded, "image/png")),
                probe: None,
            }
            .with_probe(&uri),
            markdown_block("inline", format!("![photo]({uri})"), None, cx).with_probe(&uri),
            markdown_block("invalid", format!("![photo]({bad_uri})"), None, cx),
            markdown_block(
                "unsupported",
                "![photo](data:image/unknown;base64,AA==)".into(),
                None,
                cx,
            ),
        ]
    });
    let invalid_document = blocks[2].markdown.clone().expect("invalid image document");
    let caches: Vec<_> = blocks.iter().map(|block| block.cache.clone()).collect();
    let probes: Vec<_> = blocks
        .iter()
        .filter_map(|block| block.probe.clone())
        .collect();
    let window = window(cx, blocks);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("begin data loads");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("decode data loads");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for ((id, probe), cache) in ["acp", "inline"].into_iter().zip(&probes).zip(&caches) {
            let first = assert_png(cache, probe, cx);
            window.render_frame(cx);
            let second = assert_png(cache, probe, cx);
            assert!(Arc::ptr_eq(&first, &second), "decoded data is cached");
            assert!(window.find(id).bounds().size.height >= px(HEIGHT as f32));
        }
        assert_eq!(
            window.find("image-acp").bounds().size,
            size(px(WIDTH as f32), px(HEIGHT as f32))
        );
        for (id, cache) in ["invalid", "unsupported"].into_iter().zip(&caches[2..]) {
            let errors = &cache.read(cx).errors;
            assert_eq!(errors.len(), 1);
            for (key, message) in errors {
                assert!(message.starts_with("图片无法加载"));
                assert!(!message.contains("data:"));
                assert!(window.find(format!("image-error-{id}-{key}")).visible());
            }
            assert!(window.find(id).bounds().size.height >= px(32.));
        }
    })
    .expect("data images and failures");
    cx.update(|cx| {
        invalid_document.update(cx, |state, cx| state.set_text("图片已移除", cx));
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("replace failed image with text");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            caches[2].read(cx).errors.is_empty(),
            "replaced image removes its error caption"
        );
        assert!(window.find("invalid").bounds().size.height < px(32.));
    })
    .expect("no obsolete failure");
}

#[gpui_kit::test]
fn delayed_http_image_remeasures_markdown_and_http_decode_failures_are_visible(
    cx: &mut TestAppContext,
) {
    init(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let (release, waiting) = futures::channel::oneshot::channel::<()>();
    let waiting = Arc::new(Mutex::new(Some(waiting)));
    cx.update(|cx| {
        cx.set_http_client(FakeHttpClient::create(move |request| {
            let path = request.uri().path().to_owned();
            recorded.lock().expect("requests").push(path.clone());
            let waiting = if path == "/delayed.png" {
                waiting.lock().expect("gate").take()
            } else {
                None
            };
            async move {
                if let Some(waiting) = waiting {
                    waiting.await.expect("release response");
                }
                let (status, bytes) = match path.as_str() {
                    "/delayed.png" => (200, png()),
                    "/broken.png" => (200, b"invalid PNG".to_vec()),
                    _ => (404, b"private response details".to_vec()),
                };
                Ok(Response::builder()
                    .status(status)
                    .body(bytes.into())
                    .expect("HTTP response"))
            }
        }))
    });
    let sources = [
        "https://example.com/delayed.png",
        "https://example.com/broken.png",
        "https://example.com/missing.png",
    ];
    let blocks = cx.update(|cx| {
        ["delayed", "broken", "missing"]
            .into_iter()
            .zip(sources)
            .map(|(id, source)| {
                let block = markdown_block(id, format!("![image]({source})"), None, cx);
                if id == "delayed" {
                    block.with_probe(source)
                } else {
                    block
                }
            })
            .collect::<Vec<_>>()
    });
    let caches: Vec<_> = blocks.iter().map(|block| block.cache.clone()).collect();
    let probe = blocks[0].probe.clone().expect("delayed image probe");
    let window = window(cx, blocks);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("begin requests");
    cx.run_until_parked();
    let before = cx
        .update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(caches[0].read(cx).errors.is_empty());
            assert!(
                probe
                    .frames
                    .lock()
                    .expect("image probes")
                    .last()
                    .expect("pending prepaint")
                    .is_none()
            );
            window.find("delayed").bounds().size.height
        })
        .expect("pending image");
    release.send(()).expect("pending request");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("loaded frame");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_png(&caches[0], &probe, cx);
        let after = window.find("delayed").bounds().size.height;
        assert!(after >= px(HEIGHT as f32));
        assert!(
            after > before,
            "loaded dimensions replace placeholder: {before:?} -> {after:?}"
        );
        for (id, cache) in ["broken", "missing"].into_iter().zip(&caches[1..]) {
            let errors = &cache.read(cx).errors;
            assert_eq!(errors.len(), 1);
            for (key, message) in errors {
                assert!(message.starts_with("图片无法加载"));
                assert!(!message.contains("private response details"));
                assert!(!message.contains("https://"));
                assert!(window.find(format!("image-error-{id}-{key}")).visible());
            }
        }
        window.render_frame(cx);
        assert_eq!(
            requests.lock().expect("requests").len(),
            3,
            "one load per image across repeated measurements"
        );
    })
    .expect("delayed image and errors");
}

#[test]
fn relative_images_bind_to_directory_and_file_uris_percent_decode() {
    let first = tempfile::tempdir().expect("first directory");
    let second = tempfile::tempdir().expect("second directory");
    let image = Resource::Uri("Pictures/Screen%20shot.png".into());
    assert_eq!(
        resolve(&image, Some(first.path())).expect("first directory"),
        Resource::from(first.path().join("Pictures/Screen shot.png"))
    );
    assert_eq!(
        resolve(&image, Some(second.path())).expect("second directory"),
        Resource::from(second.path().join("Pictures/Screen shot.png"))
    );
    assert!(resolve(&image, None).is_err());
    let file = first.path().join("Screen shot.png");
    let uri = url::Url::from_file_path(&file)
        .expect("file URI")
        .to_string();
    assert_eq!(
        resolve(&Resource::Uri(uri.into()), None).expect("file URI"),
        Resource::from(file)
    );
    #[cfg(not(target_os = "windows"))]
    assert!(resolve(&Resource::Uri("file://remote-host/path.png".into()), None).is_err());
    #[cfg(target_os = "windows")]
    assert_eq!(
        resolve(&Resource::Uri(r"C:\Pictures\Screen shot.png".into()), None)
            .expect("native Windows path"),
        Resource::from(PathBuf::from(r"C:\Pictures\Screen shot.png"))
    );
    assert!(resolve(&Resource::Uri("javascript:alert(1)".into()), None).is_err());
    let remote = Resource::Uri("https://example.com/image.png?v=2#preview".into());
    assert_eq!(
        resolve(&remote, None).expect("HTTP image"),
        Resource::Uri("https://example.com/image.png?v=2".into())
    );
    let svg = decode_data_uri("data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='3'%20height='2'%3E%3Crect%20width='3'%20height='2'%20fill='red'/%3E%3C/svg%3E").expect("percent encoded SVG");
    let mut app = gpui_kit::TestApp::new();
    app.update(|cx| {
        let image = svg
            .to_image_data(cx.svg_renderer())
            .expect("real SVG pixels");
        assert_eq!(image.size(0), size(DevicePixels(6), DevicePixels(4)));
        assert_eq!(
            &image.as_bytes(0).expect("SVG pixels")[..4],
            &[0, 0, 255, 255]
        );
    });
    assert!(decode_data_uri("data:image/png;base64,?").is_err());
}
