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
    image: Option<bas::acp::ImageContent>,
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
    hidden: bool,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hidden {
            return div();
        }
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
                        document,
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
    fixture_window(cx, blocks).0
}

fn fixture_window(
    cx: &mut TestAppContext,
    blocks: Vec<Block>,
) -> (WindowHandle<Root>, Entity<Fixture>) {
    let mut entity = None;
    let window = cx.open_window(size(px(500.), px(900.)), |window, cx| {
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
                hidden: false,
            }
        });
        entity = Some(fixture.clone());
        Root::new(fixture, window, cx)
    });
    (window, entity.expect("window creates fixture"))
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
                image: Some(bas::acp::ImageContent::new(encoded, "image/png")),
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
                assert!(message.starts_with("图片加载失败"));
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
        assert!(caches[2].read(cx).inline.is_empty());
        assert!(caches[2].read(cx).fallback.is_none());
        assert!(window.find("invalid").bounds().size.height < px(32.));
    })
    .expect("no obsolete failure");
}

fn inline_uri(parameter: &str) -> SharedUri {
    format!(
        "data:image/png;{parameter};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png())
    )
    .into()
}

#[gpui_kit::test]
fn embedded_assets_are_kept_until_the_last_document_releases_them(cx: &mut TestAppContext) {
    init(cx);
    let first = inline_uri("first-document");
    let second = inline_uri("second-document");
    let source = gpui_kit::ImageSource::Image(Arc::new(decode_data_uri(&first).expect("PNG")));
    let blocks = cx.update(|cx| {
        vec![
            markdown_block("first", format!("![image]({first})"), None, cx),
            markdown_block("second", format!("<img src=\"{second}\">"), None, cx),
        ]
    });
    let (window, fixture) = fixture_window(cx, blocks);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(source.is_asset_cached(cx));
    })
    .expect("native embedded image");
    for remaining in [1, 0] {
        cx.update(|cx| {
            fixture.update(cx, |fixture, cx| {
                fixture.blocks.truncate(remaining);
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .expect("release document");
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(source.is_asset_cached(cx), remaining != 0));
    }
}

#[gpui_kit::test]
fn content_replacement_cancels_obsolete_pending_loads_without_cancelling_shared_loads(
    cx: &mut TestAppContext,
) {
    init(cx);
    let (old_sender, old_receiver) = futures::channel::oneshot::channel::<()>();
    let (stable_sender, stable_receiver) = futures::channel::oneshot::channel::<()>();
    let receivers = Arc::new(Mutex::new(HashMap::<_, _>::from_iter([
        ("/old.png".to_owned(), old_receiver),
        ("/stable.png".to_owned(), stable_receiver),
    ])));
    let requests = Arc::new(Mutex::new(Vec::new()));
    cx.update(|cx| {
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |request| {
            let path = request.uri().path().to_owned();
            requests.lock().expect("requests").push(path.clone());
            let receiver = receivers.lock().expect("receivers").remove(&path);
            async move {
                receiver
                    .expect("one request per URI")
                    .await
                    .expect("response gate");
                Ok(Response::builder()
                    .status(200)
                    .body(png().into())
                    .expect("response"))
            }
        }));
    });
    let old = "https://example.invalid/old.png";
    let stable = "https://example.invalid/stable.png";
    let block = cx.update(|cx| {
        markdown_block(
            "pending",
            format!("<img src=\"{old}\"> ![stable]({stable})"),
            None,
            cx,
        )
    });
    let cache = block.cache.clone();
    let document = block.markdown.clone().expect("document");
    let window = window(cx, vec![block]);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cache.read(cx).resources.len(), 2));
    cx.update(|cx| {
        document.update(cx, |state, cx| {
            state.set_text(
                &format!("<img src=\"{stable}\"> ![again]({stable})\n\n<div>streaming"),
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert!(
        old_sender.send(()).is_err(),
        "obsolete HTTP future was cancelled"
    );
    cx.update(|cx| assert_eq!(cache.read(cx).resources.len(), 1));
    stable_sender
        .send(())
        .expect("still referenced request survives");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(cache.read(cx).errors.is_empty());
        let image = cache.read(cx).resources[&Resource::Uri(stable.into())].get();
        assert!(matches!(image, Some(Ok(_))));
        assert_eq!(requests.lock().expect("requests").len(), 2);
        // Start inline decoding and remove its owner before its task can run.
        let obsolete = inline_uri("pending");
        cache.update(cx, |cache, cx| {
            cache.start_inline(&obsolete, cx);
            assert!(cache.inline[&obsolete].result.is_none());
            cache.retain(
                HashSet::from_iter([Resource::Uri(stable.into())]),
                window,
                cx,
            );
            assert!(cache.inline.is_empty());
        });
    })
    .expect("shared request and cancelled inline task");
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cache.read(cx).inline.is_empty(),
            "cancelled task cannot repopulate cache"
        )
    });
}

#[test]
fn markdown_ownership_uses_the_parser_not_uri_substrings() {
    let source = r#"
![escaped](image\(one\).png)
![entity](image&#46;png)

`![code](not-an-image.png)`

[reference]: referenced.png
"#;
    let references = references::markdown(source).expect("GFM references");
    assert_eq!(
        references,
        HashSet::from_iter(["image(one).png", "image.png"].map(|uri| Resource::Uri(uri.into())))
    );
    assert!(
        references::markdown("images removed")
            .expect("plain text")
            .is_empty()
    );
    assert_eq!(
        references::markdown("<img src=\"image.png\">").expect("HTML image"),
        HashSet::from_iter([Resource::Uri("image.png".into())])
    );
}
