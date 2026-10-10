//! Conversation-owned image loading, including paths supplied by local agents.

use std::{collections::BTreeMap, sync::Arc};

use base64::Engine as _;
use gpui_kit::{
    EntityId, Image, ImageCache, ImageCacheError, ImageFormat, RenderImage, Resource,
    RetainAllImageCache, SharedUri, WeakEntity, img,
};

use super::*;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(super) struct Entry {
    cache: Entity<Cache>,
    _changed: Subscription,
}

struct Inline {
    result: Option<Result<Arc<RenderImage>, ImageCacheError>>,
    users: HashSet<EntityId>,
}

pub(super) struct Cache {
    owner: WeakEntity<Self>,
    directory: Option<PathBuf>,
    resources: Entity<RetainAllImageCache>,
    inline: HashMap<SharedUri, Inline>,
    errors: BTreeMap<u64, String>,
    visible: RefCell<HashSet<u64>>,
    fallback: Option<Arc<RenderImage>>,
    _tasks: Vec<Task<()>>,
}

impl Cache {
    pub(super) fn new(directory: Option<PathBuf>, cx: &mut App) -> Entity<Self> {
        let resources = RetainAllImageCache::new(cx);
        let cache = cx.new(|cx| Self {
            owner: cx.weak_entity(),
            directory,
            resources,
            inline: HashMap::new(),
            errors: BTreeMap::new(),
            visible: RefCell::new(HashSet::new()),
            fallback: None,
            _tasks: Vec::new(),
        });
        cx.observe_release(&cache, |cache, cx| {
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
        if !self.inline.contains_key(uri) {
            self.inline.insert(
                uri.clone(),
                Inline {
                    result: None,
                    users: HashSet::new(),
                },
            );
            let uri = uri.clone();
            let address = uri.clone();
            let owner = self.owner.clone();
            let renderer = cx.svg_renderer();
            let decode = cx.background_executor().spawn(async move {
                decode_data_uri(address.as_ref())
                    .and_then(|image| image.to_image_data(renderer).map_err(Into::into))
            });
            self._tasks.push(cx.spawn(async move |cx| {
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
            }));
        }
        let entry = self.inline.get_mut(uri).expect("inline image was inserted");
        if entry.result.is_none() {
            entry.users.insert(window.current_view());
        }
        entry.result.clone()
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
        self.visible.borrow_mut().insert(gpui_kit::hash(resource));
        let result = match resource {
            Resource::Uri(uri) if uri.starts_with("data:") => self.load_inline(uri, window, cx),
            _ => match resolve(resource, self.directory.as_deref()) {
                Ok(source) => self
                    .resources
                    .update(cx, |cache, cx| cache.load(&source, window, cx)),
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
        let mut images = self.images.borrow_mut();
        images
            .entry(id.to_owned())
            .or_insert_with(|| {
                let directory = self
                    .current()
                    .and_then(|session| session.cwd.as_deref().map(groups::normalize))
                    .or_else(|| self.selected_directory.clone());
                let cache = Cache::new(directory, cx);
                let changed = cx.observe(&cache, move |this, _, cx| {
                    if this
                        .conversation
                        .as_ref()
                        .is_some_and(|state| index < state.rows.len())
                    {
                        this.list.remeasure_items(index..index + 1);
                    }
                    cx.notify();
                });
                Entry {
                    cache,
                    _changed: changed,
                }
            })
            .cache
            .clone()
    }
}

pub(super) fn wrap(id: &str, cache: &Entity<Cache>, content: AnyElement, cx: &App) -> AnyElement {
    frame(id, cache, content, cx).into_any_element()
}

fn frame(id: &str, cache: &Entity<Cache>, content: AnyElement, cx: &App) -> Div {
    // A streaming block can replace its images without changing its content ID.
    cache.read(cx).visible.borrow_mut().clear();
    let used = cache.clone();
    div()
        .w_full()
        .flex()
        .flex_col()
        .image_cache(cache.clone())
        .child(content)
        .children(cache.read(cx).errors.iter().map(|(key, message)| {
            div()
                .id(SharedString::from(format!("image-error-{id}-{key}")))
                .test_support()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(message.clone())
        }))
        .on_children_prepainted(move |_, _, cx| {
            used.update(cx, |cache, cx| {
                let before = cache.errors.len();
                let visible = cache.visible.borrow();
                cache.errors.retain(|key, _| visible.contains(key));
                if cache.errors.len() != before {
                    cx.notify();
                }
            });
        })
}

pub(super) fn acp_image(
    id: &str,
    image: &basr::acp::ImageContent,
    cache: &Entity<Cache>,
    preview: bool,
    cx: &App,
) -> AnyElement {
    let source: SharedUri = format!("data:{};base64,{}", image.mime_type, image.data).into();
    let content = img(source)
        .object_fit(gpui_kit::ObjectFit::Contain)
        .id(SharedString::from(format!("image-{id}")))
        .test_support()
        .max_w_full()
        .when(preview, |image| image.size_full())
        .into_any_element();
    frame(id, cache, content, cx)
        .when(preview, |frame| frame.h_full())
        .into_any_element()
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
            .map_err(|_| ImageCacheError::Asset("文件图片地址不是本地路径".into())),
        Ok(_) => Err(ImageCacheError::Asset("不支持的图片地址协议".into())),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let path = std::path::Path::new(uri.as_ref());
            let base = if path.is_absolute() {
                std::path::Path::new("/")
            } else {
                directory
                    .ok_or_else(|| ImageCacheError::Asset("相对图片地址缺少工作目录".into()))?
            };
            let url = url::Url::from_directory_path(base)
                .map_err(|_| ImageCacheError::Asset("图片工作目录不是绝对路径".into()))?
                .join(uri.as_ref())
                .map_err(|_| ImageCacheError::Asset("图片地址无效".into()))?;
            url.to_file_path()
                .map(Resource::from)
                .map_err(|_| ImageCacheError::Asset("无法解析图片路径".into()))
        }
        Err(_) => Err(ImageCacheError::Asset("图片地址无效".into())),
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
        .ok_or_else(|| ImageCacheError::Asset("内嵌图片地址无效".into()))?;
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
        .ok_or_else(|| ImageCacheError::Asset("不支持的图片格式".into()))?;
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
            .map_err(|_| ImageCacheError::Asset("图片 Base64 数据无效".into()))?
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
            _ => return Err(ImageCacheError::Asset("图片地址的百分号编码无效".into())),
        }
    }
    Ok(decoded)
}

fn failure_message(error: &ImageCacheError) -> String {
    let detail = match error {
        ImageCacheError::BadStatus { status, .. } => format!("HTTP {status}"),
        ImageCacheError::Io(_) => "无法读取本地文件".into(),
        ImageCacheError::Asset(message) => message.to_string(),
        ImageCacheError::Image(_) | ImageCacheError::Usvg(_) => "图片数据无效".into(),
        ImageCacheError::Other(_) => "加载失败".into(),
    };
    format!("图片无法加载：{detail}")
}
