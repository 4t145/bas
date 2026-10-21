//! Disk-first artwork loading; stale bytes stay visible while HTTP revalidates.
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use futures::AsyncReadExt as _;
use gpui_kit::{
    AssetSource, Image, ImageFormat, SvgRenderer,
    http_client::{
        AsyncBody, HttpClient, RedirectPolicy, Request, RequestTimeout, http::HeaderValue,
    },
};

use super::{Artwork, FRESH_FOR, ICON_TIMEOUT, MAX_ICON_BYTES, disk};

pub(super) struct Loader {
    pub uri: String,
    pub client: Arc<dyn HttpClient>,
    pub renderer: SvgRenderer,
    pub assets: Arc<dyn AssetSource>,
    pub disk: Option<disk::Cache>,
}

pub(super) struct Ready {
    pub artwork: Artwork,
    pub entry: Option<disk::Entry>,
    pub refresh_after: Option<Duration>,
}

pub(super) fn key(uri: &str) -> String {
    match url::Url::parse(uri) {
        Ok(mut url) if matches!(url.scheme(), "http" | "https") => {
            url.set_fragment(None);
            url.into()
        }
        _ => uri.to_owned(),
    }
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Loader {
    /// # Errors
    /// Embedded assets may be missing or malformed. Disk misses/errors are optional.
    pub fn cached(&self) -> Result<Option<Ready>> {
        if !self.uri.starts_with("https://") && !self.uri.starts_with("http://") {
            let bytes = self
                .assets
                .load(&self.uri)?
                .context("embedded agent icon not found")?;
            self.renderer.parse_svg(&bytes)?;
            return Ok(Some(Ready {
                artwork: Artwork::Svg {
                    bytes: Arc::from(bytes.as_ref()),
                    embedded: true,
                },
                entry: None,
                refresh_after: None,
            }));
        }
        let Some(cache) = &self.disk else {
            return Ok(None);
        };
        match cache.read(&self.uri) {
            Ok(Some(entry)) => {
                if let Ok(artwork) = self.decode(&entry) {
                    let remaining = now()
                        .checked_sub(entry.fetched_at)
                        .map(|age| FRESH_FOR.saturating_sub(Duration::from_secs(age)))
                        .unwrap_or_default();
                    return Ok(Some(Ready {
                        artwork,
                        entry: Some(entry),
                        refresh_after: Some(remaining),
                    }));
                }
                let _ = cache.remove(&self.uri);
            }
            Err(disk::Error::Invalid) => {
                let _ = cache.remove(&self.uri);
            }
            Err(_) | Ok(None) => {}
        }
        Ok(None)
    }

    /// # Errors
    /// Reports HTTP, oversized-body and image decoding failures, never cache IO failures.
    pub async fn fetch(&self, previous: Option<Ready>) -> Result<Ready> {
        let mut request = Request::get(&self.uri).body(AsyncBody::default())?;
        if let Some(entry) = previous.as_ref().and_then(|ready| ready.entry.as_ref()) {
            for (name, value) in [
                ("if-none-match", &entry.etag),
                ("if-modified-since", &entry.last_modified),
            ] {
                if let Some(value) = value
                    .as_ref()
                    .and_then(|value| HeaderValue::from_str(value).ok())
                {
                    request.headers_mut().insert(name, value);
                }
            }
        }
        request
            .extensions_mut()
            .insert(RequestTimeout(ICON_TIMEOUT));
        request.extensions_mut().insert(RedirectPolicy::FollowAll);
        let mut response = self.client.send(request).await?;
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let etag = header("etag");
        let last_modified = header("last-modified");
        // This is an application artwork cache, not a general HTTP cache.
        // Opt-out responses can be displayed but must not survive this process.
        let persist = !response
            .headers()
            .get_all("cache-control")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .any(|directive| {
                matches!(
                    directive
                        .trim()
                        .split('=')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "no-store" | "no-cache"
                )
            });
        let (artwork, entry) = if response.status().as_u16() == 304 {
            let previous = previous.context("icon returned 304 without cached artwork")?;
            let mut entry = previous
                .entry
                .context("icon returned 304 without cached bytes")?;
            entry.fetched_at = now();
            entry.etag = etag.or(entry.etag);
            entry.last_modified = last_modified.or(entry.last_modified);
            (previous.artwork, entry)
        } else {
            if !response.status().is_success() {
                bail!("agent icon HTTP status {}", response.status());
            }
            let mime = header("content-type")
                .unwrap_or_else(|| "application/octet-stream".into())
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let mut bytes = Vec::new();
            response
                .body_mut()
                .take(MAX_ICON_BYTES + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() as u64 > MAX_ICON_BYTES {
                bail!("agent icon exceeds byte limit");
            }
            let entry = disk::Entry {
                bytes,
                mime,
                fetched_at: now(),
                etag,
                last_modified,
            };
            (self.decode(&entry)?, entry)
        };
        if let Some(cache) = &self.disk {
            let result = if persist {
                cache.write(&self.uri, &entry)
            } else {
                cache.remove(&self.uri)
            };
            if let Err(error) = result {
                tracing::debug!(%error, "Agent icon disk cache unavailable");
            }
        }
        Ok(Ready {
            artwork,
            entry: Some(entry),
            refresh_after: Some(FRESH_FOR),
        })
    }

    fn decode(&self, entry: &disk::Entry) -> Result<Artwork> {
        let url = url::Url::parse(&self.uri)?;
        let path_mime = mime_guess::from_path(url.path()).first_or_octet_stream();
        let format = ImageFormat::from_mime_type(&entry.mime)
            .or_else(|| ImageFormat::from_mime_type(path_mime.essence_str()));
        if matches!(format, Some(ImageFormat::Svg)) || format.is_none() {
            self.renderer.parse_svg(&entry.bytes)?;
            Ok(Artwork::Svg {
                bytes: Arc::from(entry.bytes.as_slice()),
                embedded: false,
            })
        } else {
            Ok(Artwork::Bitmap(
                Image::from_bytes(
                    format.context("agent icon image format unavailable")?,
                    entry.bytes.clone(),
                )
                .to_image_data(self.renderer.clone())?,
            ))
        }
    }
}
