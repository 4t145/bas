use std::{
    future::Future,
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpListener, TcpStream},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use futures::{AsyncReadExt as _, FutureExt as _, future::Either};
use gpui_kit::{Asset as _, ImageAssetLoader, ImageCacheError, Resource, SharedUri, TestApp};

const CHILD_ENV: &str = "BASRW_HTTP_IMAGE_TEST_CHILD";
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const ASSET_TIMEOUT: Duration = Duration::from_secs(8);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(40);
const SVG: &[u8] = include_bytes!("assets/image.svg");

#[test]
/// # Errors
/// Reports subprocess, fixture, or transport failures.
fn remote_svg_uses_registered_http_client() -> Result<()> {
    if std::env::var_os(CHILD_ENV).is_none() {
        return run_without_proxy();
    }

    let fixture = Fixture::start()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let client = super::client(runtime.handle())?;
    let mut app = TestApp::new();
    app.update(|cx| cx.set_http_client(Arc::clone(&client)));

    for path in ["/image.svg", "/redirect"] {
        let image = load(&mut app, &runtime, fixture.uri(path))??;
        assert_eq!(image.frame_count(), 1);
        // GPUI rasterizes SVGs at twice their intrinsic dimensions for smoothing.
        assert_eq!(
            image.size(0),
            gpui_kit::size(gpui_kit::DevicePixels(48), gpui_kit::DevicePixels(32))
        );
    }

    let missing = fixture.uri("/missing");
    match load(&mut app, &runtime, missing.clone())? {
        Err(ImageCacheError::BadStatus { uri, status, body }) => {
            assert_eq!(uri, missing);
            assert_eq!(status.as_u16(), 404);
            assert_eq!(body, "fixture image not found");
        }
        Err(error) => bail!("expected HTTP 404, got {error}"),
        Ok(_) => bail!("a missing image unexpectedly decoded"),
    }
    let mut request =
        super::Request::get(fixture.uri("/stall").as_ref()).body(super::AsyncBody::default())?;
    request
        .extensions_mut()
        .insert(super::RequestTimeout(Duration::from_millis(500)));
    let read = async {
        let mut response = client.send(request).await?;
        let mut bytes = Vec::new();
        response.body_mut().read_to_end(&mut bytes).await?;
        Ok::<_, anyhow::Error>(bytes)
    };
    ensure!(
        outside_tokio(&runtime, read)?.is_err(),
        "the caller's short deadline must bound response-body reads"
    );
    fixture.finish()?;
    Ok(())
}

/// # Errors
/// Reports child launch, exit, or deadline failures.
fn run_without_proxy() -> Result<()> {
    // Only the child environment changes; parallel tests retain their proxies.
    let test = concat!(module_path!(), "::remote_svg_uses_registered_http_client");
    let test = test
        .strip_prefix("basrw::")
        .context("unexpected test module path")?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_ENV, "1");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(name);
    }
    let mut child = command.spawn().context("spawn proxy-isolated image test")?;
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "proxy-isolated image test failed: {status}"
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            child.kill().context("kill timed-out image test")?;
            child.wait().context("reap timed-out image test")?;
            bail!("image test exceeded {PROCESS_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// # Errors
/// Reports an asset-load deadline; image errors remain in the inner result.
fn load(
    app: &mut TestApp,
    runtime: &tokio::runtime::Runtime,
    uri: SharedUri,
) -> Result<Result<Arc<gpui_kit::RenderImage>, ImageCacheError>> {
    let image = app.update(|cx| ImageAssetLoader::load(Resource::Uri(uri), cx).boxed());
    outside_tokio(runtime, image)
}

/// # Errors
/// Rejects a Tokio polling context or an operation that exceeds the test deadline.
fn outside_tokio<T>(
    runtime: &tokio::runtime::Runtime,
    operation: impl Future<Output = T>,
) -> Result<T> {
    ensure!(tokio::runtime::Handle::try_current().is_err());
    // The timer runs on Tokio, but the actual GPUI future must not: this catches
    // adapters whose response-body polling accidentally requires a Tokio context.
    let timer = runtime.spawn(async { tokio::time::sleep(ASSET_TIMEOUT).await });
    let result = futures::executor::block_on(futures::future::select(Box::pin(operation), timer));
    match result {
        Either::Left((image, timer)) => {
            timer.abort();
            Ok(image)
        }
        Either::Right((_, _)) => bail!("GPUI image load exceeded {ASSET_TIMEOUT:?}"),
    }
}

struct Fixture {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Result<Vec<String>>>>,
}

impl Fixture {
    /// # Errors
    /// Reports failure to bind or configure the loopback listener.
    fn start() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + PROCESS_TIMEOUT;
            let mut paths = Vec::new();
            while paths.len() < 5 && !stop.load(Ordering::Relaxed) {
                ensure!(Instant::now() < deadline, "fixture accept timed out");
                match listener.accept() {
                    Ok((stream, _)) => paths.push(serve(stream)?),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(paths)
        });
        Ok(Self {
            address,
            stopped,
            worker: Some(worker),
        })
    }

    fn uri(&self, path: &str) -> SharedUri {
        format!("http://{}{path}", self.address).into()
    }

    /// # Errors
    /// Reports a failed fixture worker or missing request.
    fn finish(mut self) -> Result<()> {
        let worker = self.worker.take().context("fixture worker missing")?;
        let paths = worker
            .join()
            .map_err(|_| anyhow::anyhow!("fixture panicked"))??;
        assert_eq!(
            paths,
            [
                "/image.svg",
                "/redirect",
                "/image.svg",
                "/missing",
                "/stall"
            ]
        );
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// # Errors
/// Reports malformed fixture requests or bounded socket I/O failures.
fn serve(mut stream: TcpStream) -> Result<String> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        ensure!(Instant::now() < deadline, "fixture request timed out");
        ensure!(request.len() < 8192, "fixture request headers too large");
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
    }
    let request = std::str::from_utf8(&request)?;
    let mut lines = request.lines();
    let mut start = lines
        .next()
        .context("missing request line")?
        .split_whitespace();
    ensure!(start.next() == Some("GET"), "image request must use GET");
    let path = start.next().context("missing request path")?;
    let user_agent = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value.trim());
    ensure!(
        user_agent == Some(concat!("basrw/", env!("CARGO_PKG_VERSION"))),
        "unexpected image request User-Agent: {user_agent:?}"
    );

    let (status, extra, body): (&str, &str, &[u8]) = match path {
        "/image.svg" | "/stall" => ("200 OK", "Content-Type: image/svg+xml\r\n", SVG),
        "/redirect" => ("302 Found", "Location: /image.svg\r\n", b""),
        "/missing" => (
            "404 Not Found",
            "Content-Type: text/plain\r\n",
            b"fixture image not found\n",
        ),
        _ => bail!("unexpected fixture path: {path}"),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if path == "/stall" {
        // Wait for client cancellation. If its deadline was lost, deliver a
        // successful body after the fixture timeout so the assertion fails.
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => return Ok(path.to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                return Ok(path.to_owned());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!("unexpected bytes while waiting for client cancellation"),
        }
    }
    stream.write_all(body)?;
    Ok(path.to_owned())
}
