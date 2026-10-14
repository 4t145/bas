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
                assert!(message.starts_with("图片加载失败"));
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

fn inline_uri(parameter: &str) -> SharedUri {
    format!(
        "data:image/png;{parameter};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png())
    )
    .into()
}

fn inline_result(cache: &Cache, uri: &SharedUri) -> Arc<RenderImage> {
    cache.inline[uri]
        .result
        .clone()
        .expect("inline load completed")
        .expect("decoded PNG")
}

#[gpui_kit::test]
fn replacing_streamed_markdown_and_html_images_releases_old_pixels_but_keeps_shared_images(
    cx: &mut TestAppContext,
) {
    init(cx);
    let stable = inline_uri("stable");
    let old = inline_uri("old");
    let block = cx.update(|cx| {
        markdown_block(
            "stream",
            format!("<div><img src=\"{old}\"><img src=\"{stable}\"></div>"),
            None,
            cx,
        )
    });
    let cache = block.cache.clone();
    let document = block.markdown.clone().expect("document");
    let (window, fixture) = fixture_window(cx, vec![block]);
    cx.run_until_parked();
    let (stable_image, old_image) = cx.update(|cx| {
        let cache = cache.read(cx);
        (
            inline_result(cache, &stable),
            Arc::downgrade(&inline_result(cache, &old)),
        )
    });
    for generation in 0..5 {
        let next = inline_uri(&format!("generation-{generation}"));
        cx.update(|cx| {
            document.update(cx, |state, cx| {
                state.set_text(
                    &format!(
                        "![next]({next}) <img src=\"{stable}\"> ![twice]({stable})\n\n<div><img src=\""
                    ),
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let cache = cache.read(cx);
            assert_eq!(
                cache.inline.len(),
                2,
                "only current resource owners survive"
            );
            assert!(Arc::ptr_eq(&stable_image, &inline_result(cache, &stable)));
            assert!(cache.inline.contains_key(&next));
        })
        .expect("replacement frame");
    }
    cx.run_until_parked();
    assert!(
        old_image.upgrade().is_none(),
        "obsolete decoded pixels are released"
    );

    cx.update(|cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.hidden = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            cache.read(cx).inline.len(),
            2,
            "invisibility is not deletion"
        );
    })
    .expect("collapsed content");
    cx.update(|cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.hidden = false;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(Arc::ptr_eq(
            &stable_image,
            &inline_result(cache.read(cx), &stable)
        ));
    });

    drop(stable_image);
    let weak_cache = cache.downgrade();
    let pixels = cx.update(|cx| Arc::downgrade(&inline_result(cache.read(cx), &stable)));
    cx.update(|cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.blocks.clear();
            fixture._changed.clear();
            cx.notify();
        })
    });
    drop(document);
    drop(cache);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("release rendered owners");
    cx.run_until_parked();
    assert!(
        weak_cache.upgrade().is_none(),
        "cache ownership is released"
    );
    assert!(
        pixels.upgrade().is_none(),
        "cache release drops decoded pixels"
    );
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

#[gpui_kit::test]
fn releasing_cache_cancels_pending_requests_and_releases_error_fallback(cx: &mut TestAppContext) {
    init(cx);
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    let receiver = Arc::new(Mutex::new(Some(receiver)));
    cx.update(|cx| {
        cx.set_http_client(FakeHttpClient::create(move |_| {
            let receiver = receiver
                .lock()
                .expect("receiver")
                .take()
                .expect("single request");
            async move {
                receiver.await.expect("response gate");
                Ok(Response::builder()
                    .status(200)
                    .body(png().into())
                    .expect("response"))
            }
        }));
    });
    let cache = cx.update(|cx| Cache::new(None, cx));
    let weak = cache.downgrade();
    let fallback = cx.update(|cx| {
        cache.update(cx, |cache, cx| {
            let source = Resource::Uri("https://example.invalid/pending.png".into());
            cache
                .resources
                .insert(source.clone(), ImageCacheItem::new(&source, cx));
            let _ = cache.failed(
                &Resource::Uri("data:image/png;base64,broken".into()),
                ImageCacheError::Asset("invalid image".into()),
                cx,
            );
            Arc::downgrade(cache.fallback.as_ref().expect("fallback"))
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cache.update(cx, |cache, cx| {
            cache.start_inline(&inline_uri("cancel-before-decode"), cx);
            assert!(cache.inline.values().all(|entry| entry.result.is_none()));
        })
    });
    cx.update(|_| drop(cache));
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    assert!(fallback.upgrade().is_none());
    assert!(
        sender.send(()).is_err(),
        "cache release cancels pending HTTP load"
    );
}

#[test]
fn markdown_ownership_uses_the_parser_not_uri_substrings() {
    let source = r#"
![escaped](image\(one\).png)
![entity](image&#46;png)

`![code](not-an-image.png)`

[reference]: referenced.png
"#;
    let references = markdown_references(source).expect("GFM references");
    assert_eq!(
        references,
        HashSet::from_iter(["image(one).png", "image.png"].map(|uri| Resource::Uri(uri.into())))
    );
    assert!(
        markdown_references("images removed")
            .expect("plain text")
            .is_empty()
    );
    assert_eq!(
        markdown_references("<img src=\"image.png\">").expect("HTML image"),
        HashSet::from_iter([Resource::Uri("image.png".into())])
    );
}

#[test]
fn html_and_markdown_ownership_respects_entities_code_and_incomplete_streams() {
    let source = r#"
![markdown](markdown&#46;png)

<div><IMG SRC="html&#46;png"><img src='query.png?a=1&amp;b=2'>
<!-- <img src="comment.png"> -->
<script>const text = '<img src="script.png">';</script>
</div>

`<img src="inline-code.png">`

```html
<img src="fenced-code.png">
```

<div><img src="complete.png"><img src="incomplete.png
"#;
    assert_eq!(
        markdown_references(source).expect("mixed content"),
        HashSet::from_iter(
            [
                "markdown.png",
                "html.png",
                "query.png?a=1&b=2",
                "complete.png"
            ]
            .map(|uri| Resource::Uri(uri.into()))
        )
    );
    assert!(
        markdown_references("<div>images removed</div>")
            .expect("HTML without images")
            .is_empty()
    );
}

#[test]
fn only_used_image_definitions_own_resources() {
    let source = r#"
![full][Photo]
![collapsed][]
![shortcut]
[ordinary link][link]
![escaped][a\*b]

[photo]: full&#46;png
[collapsed]: collapsed.png
[shortcut]: shortcut.png
[a\*b]: escaped\(name\).png
[unused]: unused.png
[link]: link.png
"#;
    assert_eq!(
        markdown_references(source).expect("reference definitions"),
        HashSet::from_iter(
            [
                "full.png",
                "collapsed.png",
                "shortcut.png",
                "escaped(name).png"
            ]
            .map(|uri| Resource::Uri(uri.into()))
        )
    );
}

#[test]
fn html_ownership_retains_minifier_attribute_aliases_without_retaining_unrelated_images() {
    let references = markdown_references(
        "<div><img src='  space   name.png  '><img src='double&amp;amp;.png'></div>",
    )
    .expect("HTML attributes");
    assert_eq!(
        references,
        HashSet::from_iter(
            [
                "  space   name.png  ",
                "space name.png",
                "double&amp;.png",
                "double&.png",
            ]
            .map(|uri| Resource::Uri(uri.into()))
        )
    );
}

#[gpui_kit::test]
fn html_attribute_aliases_match_rendered_images_and_release_with_their_sources(
    cx: &mut TestAppContext,
) {
    init(cx);
    let directory = tempfile::tempdir().expect("HTML image directory");
    let names = ["space name.png", "double&.png"];
    for name in names {
        std::fs::write(directory.path().join(name), png()).expect("local PNG");
    }
    let block = cx.update(|cx| {
        markdown_block(
            "html",
            "<div><img src='  space   name.png  '><img src='double&amp;amp;.png'></div>".into(),
            Some(directory.path().to_owned()),
            cx,
        )
    });
    let cache = block.cache.clone();
    let document = block.markdown.clone().expect("document");
    let window = window(cx, vec![block]);
    cx.run_until_parked();
    let before = cx.update(|cx| {
        names.map(|name| {
            cache.read(cx).resources[&Resource::from(directory.path().join(name))]
                .get()
                .expect("image completed")
                .expect("decoded image")
        })
    });
    cx.update(|cx| {
        document.update(cx, |state, cx| {
            state.set_text(
                "<div><img src='  space   name.png  '><img src='double&amp;amp;.png'>updated</div>",
                cx,
            );
        });
    });
    cx.run_until_parked();
    let pixels = cx
        .update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let cache = cache.read(cx);
            assert!(cache.errors.is_empty());
            assert_eq!(cache.resources.len(), names.len());
            names.map(|name| {
                let source = Resource::from(directory.path().join(name));
                let image = cache.resources[&source]
                    .get()
                    .expect("image completed")
                    .expect("decoded image");
                assert!(
                    before.iter().any(|previous| Arc::ptr_eq(previous, &image)),
                    "content updates retain the already decoded HTML image"
                );
                Arc::downgrade(&image)
            })
        })
        .expect("render HTML attribute aliases");
    drop(before);
    cx.update(|cx| {
        document.update(cx, |state, cx| {
            state.set_text("<div>images removed</div>", cx)
        });
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(cache.read(cx).resources.is_empty());
    })
    .expect("HTML replacement releases resources");
    cx.run_until_parked();
    assert!(pixels.iter().all(|image| image.upgrade().is_none()));
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
