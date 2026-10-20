//! Conversation-owned image loading, including paths supplied by local agents.

use std::{collections::BTreeMap, sync::Arc};

use base64::Engine as _;
use gds_uim::el;
use gpui_kit::base::{Text, TextViewState};
use gpui_kit::{
    EntityId, Image, ImageCache, ImageCacheError, ImageCacheItem, ImageFormat, RenderImage,
    Resource, SharedUri, WeakEntity, img,
};

use super::*;

mod embedded;
mod references;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(super) struct Entry {
    cache: Entity<Cache>,
    _changed: Subscription,
}

struct Inline {
    result: Option<Result<Arc<RenderImage>, ImageCacheError>>,
    users: HashSet<EntityId>,
    _task: Task<()>,
}

pub(super) struct Cache {
    owner: WeakEntity<Self>,
    directory: Option<PathBuf>,
    resources: HashMap<Resource, ImageCacheItem>,
    inline: HashMap<SharedUri, Inline>,
    errors: BTreeMap<u64, String>,
    fallback: Option<Arc<RenderImage>>,
    content: Option<u64>,
    embedded: embedded::Owners,
}

impl Cache {
    pub(super) fn new(directory: Option<PathBuf>, cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|cx| Self {
            owner: cx.weak_entity(),
            directory,
            resources: HashMap::new(),
            inline: HashMap::new(),
            errors: BTreeMap::new(),
            fallback: None,
            content: None,
            embedded: embedded::Owners::default(),
        });
        cx.observe_release(&cache, |cache, cx| {
            cache.embedded.release(cx);
            for entry in cache.resources.values() {
                if let Some(Ok(image)) = entry.get() {
                    cx.drop_image(image, None);
                }
            }
            for entry in cache.inline.values() {
                if let Some(Ok(image)) = &entry.result {
                    cx.drop_image(image.clone(), None);
                }
            }
            if let Some(image) = cache.fallback.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        cache
    }

    fn load_inline(
        &mut self,
        uri: &SharedUri,
        window: &Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.start_inline(uri, cx);
        let entry = self.inline.get_mut(uri).expect("inline image was inserted");
        if entry.result.is_none() {
            entry.users.insert(window.current_view());
        }
        entry.result.clone()
    }

    fn start_inline(&mut self, uri: &SharedUri, cx: &mut App) {
        if !self.inline.contains_key(uri) {
            let key = uri.clone();
            let uri = key.clone();
            let address = uri.clone();
            let owner = self.owner.clone();
            let renderer = cx.svg_renderer();
            let decode = cx.background_executor().spawn(async move {
                decode_data_uri(address.as_ref())
                    .and_then(|image| image.to_image_data(renderer).map_err(Into::into))
            });
            let task = cx.spawn(async move |cx| {
                let result = decode.await;
                let _ = owner.update(cx, |cache, cx| {
                    if let Some(entry) = cache.inline.get_mut(&uri) {
                        entry.result = Some(result);
                        for user in entry.users.drain() {
                            App::notify(cx, user);
                        }
                    }
                    cx.notify();
                });
            });
            self.inline.insert(
                key,
                Inline {
                    result: None,
                    users: HashSet::new(),
                    _task: task,
                },
            );
        }
    }

    fn retain(&mut self, references: HashSet<Resource>, window: &mut Window, cx: &mut App) {
        let errors: HashSet<_> = references.iter().map(gpui_kit::hash).collect();
        let resources: HashSet<_> = references
            .iter()
            .filter(|resource| !matches!(resource, Resource::Uri(uri) if uri.starts_with("data:")))
            .filter_map(|resource| resolve(resource, self.directory.as_deref()).ok())
            .collect();
        self.resources.retain(|resource, entry| {
            if resources.contains(resource) {
                return true;
            }
            if let Some(Ok(image)) = entry.get() {
                cx.drop_image(image, Some(window));
            }
            false
        });
        self.inline.retain(|uri, entry| {
            if references.contains(&Resource::Uri(uri.clone())) {
                return true;
            }
            if let Some(Ok(image)) = &entry.result {
                cx.drop_image(image.clone(), Some(window));
            }
            false
        });
        let before = self.errors.len();
        self.errors.retain(|key, _| errors.contains(key));
        if self.errors.is_empty()
            && let Some(image) = self.fallback.take()
        {
            cx.drop_image(image, Some(window));
        }
        if self.errors.len() != before {
            cx.notify(self.owner.entity_id());
        }
    }

    fn markdown_changed(&mut self, source: &str, window: &mut Window, cx: &mut App) {
        let content = gpui_kit::hash(&source);
        if self.content != Some(content) {
            self.content = Some(content);
            if let Some(references) = markdown_references(source) {
                self.embedded
                    .update(&references, self.owner.entity_id(), cx);
                self.retain(references, window, cx);
            }
        }
        // TextView renders embedded bytes directly, bypassing ImageCache. Keep
        // application validation/error reporting and document ownership explicit.
        for uri in self.embedded.uris() {
            let _ = self.load(&Resource::Uri(uri), window, cx);
        }
    }

    fn image_changed(&mut self, source: &SharedUri, window: &mut Window, cx: &mut App) {
        self.content = None;
        self.embedded.release(cx);
        self.retain(
            HashSet::from_iter([Resource::Uri(source.clone())]),
            window,
            cx,
        );
    }

    fn failed(
        &mut self,
        resource: &Resource,
        error: ImageCacheError,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = gpui_kit::hash(resource);
        if let std::collections::btree_map::Entry::Vacant(entry) = self.errors.entry(key) {
            entry.insert(failure_message(&error));
            cx.notify(self.owner.entity_id());
        }
        if self.fallback.is_none() {
            self.fallback = cx
                .svg_renderer()
                .render_single_frame(include_bytes!("images/assets/unavailable.svg"), 1.)
                .ok();
        }
        Some(self.fallback.clone().ok_or(error))
    }
}

impl ImageCache for Cache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let result = match resource {
            Resource::Uri(uri) if uri.starts_with("data:") => self.load_inline(uri, window, cx),
            _ => match resolve(resource, self.directory.as_deref()) {
                Ok(source) => self
                    .resources
                    .entry(source.clone())
                    .or_insert_with(|| ImageCacheItem::new(&source, cx))
                    .use_image(window),
                Err(error) => Some(Err(error)),
            },
        };
        match result {
            Some(Err(error)) => self.failed(resource, error, cx),
            result => result,
        }
    }
}

impl Workspace {
    pub(super) fn content_images(
        &self,
        id: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Entity<Cache> {
        let mut images = self.selected.timeline.images.borrow_mut();
        images
            .entry(id.to_owned())
            .or_insert_with(|| {
                let directory = self
                    .current()
                    .and_then(|session| session.cwd.as_deref().map(navigation::groups::normalize))
                    .or_else(|| self.navigation.selected_directory.clone());
                let cache = Cache::new(directory, cx);
                let changed = cx.observe(&cache, move |this, _, cx| {
                    this.on_content_images_changed(index, cx);
                });
                Entry {
                    cache,
                    _changed: changed,
                }
            })
            .cache
            .clone()
    }

    fn on_content_images_changed(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .selected
            .timeline
            .conversation
            .as_ref()
            .is_some_and(|state| index < state.rows.len())
        {
            self.selected
                .timeline
                .list
                .remeasure_items(index..index + 1);
        }
        cx.notify();
    }
}

pub(super) fn wrap(
    id: &str,
    cache: &Entity<Cache>,
    document: &Entity<TextViewState>,
    content: AnyElement,
    cx: &App,
) -> AnyElement {
    // Use the displayed parse, not the newest streaming input: parsing may lag.
    let source = Text::from(TextView::new(document)).get_text(cx);
    let used = cache.clone();
    el! {
        #frame(id, cache, content, cx) {
            on_children_prepainted: $move |_, window, cx| {
                used.update(cx, |cache, cx| cache.markdown_changed(&source, window, cx));
            };
        }
    }
    .into_any_element()
}

fn frame(id: &str, cache: &Entity<Cache>, content: AnyElement, cx: &App) -> Div {
    el! {
        #div {
            w_full; flex; flex_col;
            image_cache: $cache.clone();
        } [
            $content,
            ..$cache.read(cx).errors.iter().map(|(key, message)| crate::ui::feedback(
                format!("image-error-{id}-{key}"), message.clone(), cx.theme().muted_foreground,
            )),
        ]
    }
}

pub(super) fn acp_image(
    id: &str,
    image: &bas::acp::ImageContent,
    cache: &Entity<Cache>,
    preview: bool,
    cx: &App,
) -> AnyElement {
    let source: SharedUri = format!("data:{};base64,{}", image.mime_type, image.data).into();
    let content = el! {
        #img($source.clone()) {
            object_fit: gpui_kit::ObjectFit::Contain;
            id: $SharedString::from(format!("image-{id}"));
            test_support;
            max_w_full;
            @preview => { size_full; };
        }
    }
    .into_any_element();
    let used = cache.clone();
    el! {
        #frame(id, cache, content, cx) {
            on_children_prepainted: $move |_, window, cx| {
                used.update(cx, |cache, cx| cache.image_changed(&source, window, cx));
            };
            @preview => { h_full; };
        }
    }
    .into_any_element()
}

fn markdown_references(source: &str) -> Option<HashSet<Resource>> {
    references::markdown(source)
}

/// Resolves image addresses using the conversation's captured working directory.
///
/// # Errors
/// Returns an error for unsupported addresses or paths without a valid base directory.
fn resolve(
    resource: &Resource,
    directory: Option<&std::path::Path>,
) -> Result<Resource, ImageCacheError> {
    let Resource::Uri(uri) = resource else {
        return Ok(resource.clone());
    };
    #[cfg(target_os = "windows")]
    if std::path::Path::new(uri.as_ref()).is_absolute() {
        return Ok(Resource::from(PathBuf::from(uri.as_ref())));
    }
    match url::Url::parse(uri.as_ref()) {
        Ok(mut url) if matches!(url.scheme(), "http" | "https") => {
            url.set_fragment(None);
            Ok(Resource::Uri(url.as_str().to_owned().into()))
        }
        Ok(url) if url.scheme() == "file" => url
            .to_file_path()
            .map(Resource::from)
            .map_err(|_| ImageCacheError::Asset(crate::i18n::tr!("image-local-path").into())),
        Ok(_) => Err(ImageCacheError::Asset(
            crate::i18n::tr!("image-protocol").into(),
        )),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let path = std::path::Path::new(uri.as_ref());
            let base = if path.is_absolute() {
                std::path::Path::new("/")
            } else {
                directory.ok_or_else(|| {
                    ImageCacheError::Asset(crate::i18n::tr!("image-no-directory").into())
                })?
            };
            let url = url::Url::from_directory_path(base)
                .map_err(|_| {
                    ImageCacheError::Asset(crate::i18n::tr!("image-directory-relative").into())
                })?
                .join(uri.as_ref())
                .map_err(|_| {
                    ImageCacheError::Asset(crate::i18n::tr!("image-invalid-address").into())
                })?;
            url.to_file_path()
                .map(Resource::from)
                .map_err(|_| ImageCacheError::Asset(crate::i18n::tr!("image-invalid-path").into()))
        }
        Err(_) => Err(ImageCacheError::Asset(
            crate::i18n::tr!("image-invalid-address").into(),
        )),
    }
}

/// Decodes an embedded image into its original encoded bytes.
///
/// # Errors
/// Returns an error for unsupported image formats or malformed URI and Base64 data.
fn decode_data_uri(uri: &str) -> Result<Image, ImageCacheError> {
    let (description, data) = uri
        .strip_prefix("data:")
        .and_then(|uri| uri.split_once(','))
        .ok_or_else(|| ImageCacheError::Asset(crate::i18n::tr!("image-invalid-data-url").into()))?;
    let mime = description
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let format = ImageFormat::from_mime_type(&mime)
        .or_else(|| {
            matches!(mime.as_str(), "image/x-icon" | "image/vnd.microsoft.icon")
                .then_some(ImageFormat::Ico)
        })
        .ok_or_else(|| ImageCacheError::Asset(crate::i18n::tr!("image-format").into()))?;
    let bytes = if description
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        let data: Vec<u8> = percent_decode(data)?
            .into_iter()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(&data)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&data))
            .map_err(|_| ImageCacheError::Asset(crate::i18n::tr!("image-invalid-base64").into()))?
    } else {
        percent_decode(data)?
    };
    Ok(Image::from_bytes(format, bytes))
}

/// Decodes URI bytes without treating literal plus signs as spaces.
///
/// # Errors
/// Returns an error when a percent escape does not contain two hexadecimal digits.
fn percent_decode(data: &str) -> Result<Vec<u8>, ImageCacheError> {
    let mut decoded = Vec::with_capacity(data.len());
    let mut bytes = data.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let high = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
        let low = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
        match (high, low) {
            (Some(high), Some(low)) => decoded.push((high * 16 + low) as u8),
            _ => {
                return Err(ImageCacheError::Asset(
                    crate::i18n::tr!("image-invalid-encoding").into(),
                ));
            }
        }
    }
    Ok(decoded)
}

fn failure_message(error: &ImageCacheError) -> String {
    let detail = match error {
        ImageCacheError::BadStatus { status, .. } => format!("HTTP {status}"),
        ImageCacheError::Io(_) => crate::i18n::tr!("image-read-failed").into(),
        ImageCacheError::Asset(message) => message.to_string(),
        ImageCacheError::Image(_) | ImageCacheError::Usvg(_) => {
            crate::i18n::tr!("image-invalid-data").into()
        }
        ImageCacheError::Other(_) => crate::i18n::tr!("load-failed").into(),
    };
    crate::i18n::tr!("image-load-error", detail = (detail).to_string())
}
