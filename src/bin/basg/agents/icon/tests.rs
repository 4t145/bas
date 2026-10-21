use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gpui_kit::{
    Context, Render, TestAppContext,
    component::Root,
    http_client::{FakeHttpClient, Response},
    size,
    test::TestWindowExt,
};

use super::*;

const URI: &str = "https://example.invalid/agent.svg?v=1";
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M0 0h24v24H0z"/></svg>"#;

fn entry(fetched_at: u64) -> disk::Entry {
    disk::Entry {
        bytes: SVG.as_bytes().to_vec(),
        mime: "image/svg+xml".into(),
        fetched_at,
        etag: Some("\"one\"".into()),
        last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".into()),
    }
}

fn init(cx: &mut TestAppContext) -> (tempfile::TempDir, disk::Cache) {
    let directory = tempfile::tempdir().expect("temporary cache");
    let disk = disk::Cache::at(directory.path().join("icons"));
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
        cx.set_global(Cache {
            entries: HashMap::new(),
            disk: Some(disk.clone()),
        });
    });
    (directory, disk)
}

fn svg_bytes(cx: &mut TestAppContext, uri: &str) -> Vec<u8> {
    cx.update(|cx| match cached(&uri.to_owned().into(), cx) {
        Some(Artwork::Svg {
            bytes,
            embedded: false,
        }) => bytes.to_vec(),
        _ => panic!("expected remote SVG"),
    })
}

struct Fixture;
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(AgentIcon::new("artwork", URI))
    }
}

#[gpui_kit::test]
fn windows_share_download_and_restart_reuses_disk_without_network(cx: &mut TestAppContext) {
    let (_directory, disk) = init(cx);
    let requests = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |_| {
            requests.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(Response::builder()
                    .header("content-type", "image/svg+xml")
                    .body(SVG.into())
                    .expect("response"))
            }
        }));
    });
    let windows = [0, 1].map(|_| {
        cx.open_window(size(px(200.), px(100.)), |window, cx| {
            let fixture = cx.new(|_| Fixture);
            Root::new(fixture, window, cx)
        })
    });
    cx.run_until_parked();
    for handle in windows {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("artwork-svg").visible());
        })
        .expect("render icon");
    }
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        disk.read(URI).expect("disk").expect("saved").bytes,
        SVG.as_bytes()
    );
    cx.update(|cx| {
        cx.set_global(Cache {
            entries: HashMap::new(),
            disk: Some(disk.clone()),
        })
    });
    cx.update(|cx| {
        cached(&format!("{URI}#same-image").into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    cx.update(|cx| {
        cached(&"https://example.invalid/agent.svg?v=2".into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn stale_artwork_is_visible_before_conditional_refresh_and_304_renews_it(cx: &mut TestAppContext) {
    let (_directory, disk) = init(cx);
    disk.write(URI, &entry(1)).expect("old artwork");
    let (send, receive) = futures::channel::oneshot::channel();
    let receive = Arc::new(Mutex::new(Some(receive)));
    cx.update(|cx| {
        cx.set_http_client(FakeHttpClient::create(move |request| {
            assert_eq!(request.headers()["if-none-match"], "\"one\"");
            assert_eq!(
                request.headers()["if-modified-since"],
                "Mon, 01 Jan 2024 00:00:00 GMT"
            );
            let receive = receive.lock().expect("gate").take().expect("one request");
            async move {
                receive.await.expect("released");
                Ok(Response::builder()
                    .status(304)
                    .header("etag", "\"two\"")
                    .body("".into())
                    .expect("response"))
            }
        }))
    });
    cx.update(|cx| {
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    assert_eq!(disk.read(URI).expect("disk").expect("old").fetched_at, 1);
    send.send(()).expect("release");
    cx.run_until_parked();
    let saved = disk.read(URI).expect("disk").expect("renewed");
    assert!(saved.fetched_at > 1);
    assert_eq!(saved.etag.as_deref(), Some("\"two\""));
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
}

#[gpui_kit::test]
fn failed_refresh_keeps_old_artwork_and_retries_after_cooldown(cx: &mut TestAppContext) {
    let (_directory, disk) = init(cx);
    disk.write(URI, &entry(1)).expect("old artwork");
    let requests = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |_| {
            let attempt = requests.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(Response::builder()
                    .status(if attempt == 0 { 503 } else { 200 })
                    .header("content-type", "image/svg+xml")
                    .body(SVG.into())
                    .expect("response"))
            }
        }));
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    assert_eq!(disk.read(URI).expect("disk").expect("old").fetched_at, 1);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    cx.executor().advance_clock(RETRY_AFTER);
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    cx.run_until_parked();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert!(disk.read(URI).expect("disk").expect("updated").fetched_at > 1);
}

#[gpui_kit::test]
fn invalid_and_oversized_replacements_never_overwrite_valid_cache(cx: &mut TestAppContext) {
    let (_directory, disk) = init(cx);
    let requests = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |_| {
            let attempt = requests.fetch_add(1, Ordering::SeqCst);
            async move {
                let bytes = if attempt == 0 {
                    b"not svg".to_vec()
                } else {
                    vec![b'x'; MAX_ICON_BYTES as usize + 1]
                };
                Ok(Response::builder()
                    .header("content-type", "image/svg+xml")
                    .body(bytes.into())
                    .expect("response"))
            }
        }));
    });
    for _ in 0..2 {
        disk.write(URI, &entry(1)).expect("seed");
        cx.update(|cx| {
            cx.set_global(Cache {
                entries: HashMap::new(),
                disk: Some(disk.clone()),
            });
            cached(&URI.into(), cx);
        });
        cx.run_until_parked();
        assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
        assert_eq!(disk.read(URI).expect("disk").expect("unchanged"), entry(1));
    }
}

#[gpui_kit::test]
fn corrupt_artwork_refetches_and_no_store_removes_persistent_copy(cx: &mut TestAppContext) {
    let (_directory, disk) = init(cx);
    let mut corrupt = entry(loading::now());
    corrupt.bytes = b"invalid svg".to_vec();
    disk.write(URI, &corrupt).expect("corrupt artwork fixture");
    cx.update(|cx| {
        cx.set_http_client(FakeHttpClient::create(|request| async move {
            assert!(!request.headers().contains_key("if-none-match"));
            Ok(Response::builder()
                .header("content-type", "image/svg+xml")
                .header("cache-control", "private, no-store")
                .body(SVG.into())
                .expect("response"))
        }));
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    assert!(disk.read(URI).expect("disk").is_none());
}

#[gpui_kit::test]
fn unavailable_cache_does_not_block_bitmap_and_local_paths_are_not_read(cx: &mut TestAppContext) {
    let (directory, _) = init(cx);
    let blocker = directory.path().join("not-a-directory");
    std::fs::write(&blocker, SVG).expect("blocked cache");
    let requests = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        cx.global_mut::<Cache>().disk = Some(disk::Cache::at(blocker.join("icons")));
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |_| {
            requests.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(Response::builder()
                    .header("content-type", "image/png")
                    .body(
                        include_bytes!("../../workspace/images/assets/sample.png")
                            .to_vec()
                            .into(),
                    )
                    .expect("response"))
            }
        }));
        cached(&"https://example.invalid/agent.png".into(), cx);
        cached(&blocker.to_string_lossy().to_string().into(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cached(&"https://example.invalid/agent.png".into(), cx),
            Some(Artwork::Bitmap(_))
        ));
        assert!(cached(&blocker.to_string_lossy().to_string().into(), cx).is_none());
    });
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn locked_disk_cache_does_not_prevent_network_display(cx: &mut TestAppContext) {
    let (directory, disk) = init(cx);
    disk.write(URI, &entry(1)).expect("old cache");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("icons/.lock"))
        .expect("cache lock");
    lock.lock().expect("peer holds lock");
    cx.update(|cx| {
        cx.set_http_client(FakeHttpClient::create(|_| async {
            Ok(Response::builder()
                .header("content-type", "image/svg+xml")
                .body(SVG.into())
                .expect("response"))
        }));
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(svg_bytes(cx, URI), SVG.as_bytes());
    drop(lock);
    assert_eq!(
        disk.read(URI).expect("disk").expect("unchanged").fetched_at,
        1
    );
}

#[gpui_kit::test]
fn hung_initial_request_times_out_and_can_retry(cx: &mut TestAppContext) {
    let (_directory, _) = init(cx);
    let requests = Arc::new(AtomicUsize::new(0));
    cx.update(|cx| {
        let requests = requests.clone();
        cx.set_http_client(FakeHttpClient::create(move |_| {
            requests.fetch_add(1, Ordering::SeqCst);
            std::future::pending()
        }));
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(ICON_TIMEOUT);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(cached(&URI.into(), cx).is_none());
        assert!(
            cx.global::<Cache>().entries[&SharedString::from(URI)]
                .task
                .is_none()
        );
    });
    cx.executor().advance_clock(RETRY_AFTER);
    cx.update(|cx| {
        cached(&URI.into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}
