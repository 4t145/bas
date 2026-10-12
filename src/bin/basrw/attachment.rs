use std::{fs, io::Read, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use base64::Engine;
use basr::acp;

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Default)]
pub(super) struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl Draft {
    pub fn matches(&self, text: &str, attachments: &[Attachment]) -> bool {
        self.text == text
            && self
                .attachments
                .iter()
                .map(|item| item.id)
                .eq(attachments.iter().map(|item| item.id))
    }
}

#[derive(Clone, Debug)]
pub(super) struct Attachment {
    pub id: uuid::Uuid,
    pub name: String,
    pub mention: String,
    pub label: String,
    pub bytes: usize,
    pub content: Arc<acp::ContentBlock>,
}

impl Attachment {
    /// # Errors
    /// Rejects unreadable, non-regular, or oversized files and invalid file URLs.
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path).with_context(|| {
            crate::i18n::tr!("file-read-failed", path = (path.display()).to_string())
        })?;
        if !metadata.is_file() {
            bail!(crate::i18n::tr!(
                "attachment-regular-file",
                path = path.display().to_string()
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            bail!(crate::i18n::tr!(
                "attachment-file-limit",
                path = path.display().to_string()
            ));
        }
        let mut data = Vec::new();
        fs::File::open(path)?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_FILE_BYTES {
            bail!(crate::i18n::tr!(
                "attachment-file-limit",
                path = path.display().to_string()
            ));
        }
        let bytes = data.len();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let uri = url::Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!(crate::i18n::tr!("attachment-absolute-path")))?
            .to_string();
        let mime = mime_guess::from_path(path).first_raw();
        let mime = mime.unwrap_or("application/octet-stream").to_owned();
        let content = if mime.starts_with("image/") {
            acp::ContentBlock::Image(acp::ImageContent::new(
                base64::engine::general_purpose::STANDARD.encode(data),
                mime,
            ))
        } else {
            let text = std::str::from_utf8(&data)
                .ok()
                .filter(|text| !text.contains('\0') && mime != "application/pdf");
            let resource = match text {
                Some(text) => {
                    let mime = if path.extension().is_some_and(|ext| ext == "nix") {
                        "text/x-nix".into()
                    } else if mime == "application/octet-stream" {
                        "text/plain".into()
                    } else {
                        mime
                    };
                    acp::EmbeddedResourceResource::TextResourceContents(
                        acp::TextResourceContents::new(text, uri).mime_type(mime),
                    )
                }
                None => acp::EmbeddedResourceResource::BlobResourceContents(
                    acp::BlobResourceContents::new(
                        base64::engine::general_purpose::STANDARD.encode(&data),
                        uri,
                    )
                    .mime_type(mime),
                ),
            };
            acp::ContentBlock::Resource(acp::EmbeddedResource::new(resource))
        };
        let label = if matches!(content, acp::ContentBlock::Image(_)) {
            crate::i18n::tr!("attachment-image").to_owned()
        } else {
            compact_label(&name)
        };
        Ok(Self {
            id: uuid::Uuid::new_v4(),
            mention: marker(&label),
            label,
            name,
            bytes,
            content: Arc::new(content),
        })
    }

    /// # Errors
    /// Rejects clipboard images larger than the attachment limit.
    pub fn image(image: &gpui_kit::Image) -> Result<Self> {
        let bytes = image.bytes().len();
        if bytes as u64 > MAX_FILE_BYTES {
            bail!(crate::i18n::tr!("attachment-size-limit"));
        }
        let label = crate::i18n::tr!("attachment-image");
        let name = format!("{label}.{}", image.format().extension());
        Ok(Self {
            id: uuid::Uuid::new_v4(),
            mention: marker(label),
            label: label.into(),
            name,
            bytes,
            content: Arc::new(acp::ContentBlock::Image(acp::ImageContent::new(
                base64::engine::general_purpose::STANDARD.encode(image.bytes()),
                image.format().mime_type(),
            ))),
        })
    }

    pub fn set_label(&mut self, label: String) {
        self.mention = marker(&label);
        self.label = label;
    }

    pub fn supported(&self, capabilities: &acp::PromptCapabilities) -> bool {
        match self.content.as_ref() {
            acp::ContentBlock::Image(_) => capabilities.image,
            acp::ContentBlock::Resource(resource) => {
                capabilities.embedded_context
                    || matches!(
                        resource.resource,
                        acp::EmbeddedResourceResource::TextResourceContents(_)
                    )
            }
            _ => false,
        }
    }

    fn for_prompt(&self, capabilities: &acp::PromptCapabilities) -> acp::ContentBlock {
        if !capabilities.embedded_context
            && let acp::ContentBlock::Resource(resource) = self.content.as_ref()
            && let acp::EmbeddedResourceResource::TextResourceContents(text) = &resource.resource
        {
            return format!(
                "File: {}\nURI: {}\n\n{}",
                self.name,
                text.uri,
                file_code(&text.text, &text.uri)
            )
            .into();
        }
        self.content.as_ref().clone()
    }
}

/// File contents remain literal even when a text-only backend echoes them as Markdown.
pub(super) fn file_code(text: &str, uri: &str) -> String {
    let fence = "`".repeat(
        text.split(|ch| ch != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    let language = Path::new(uri)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| ext.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .unwrap_or_default();
    format!("{fence}{language}\n{text}\n{fence}")
}

pub(super) struct Mention<'a> {
    pub range: std::ops::Range<usize>,
    pub attachment: &'a Attachment,
}

/// The visible markers live in the editor's undo history; their payloads stay in the draft.
pub(super) fn mentions<'a>(text: &str, attachments: &'a [Attachment]) -> Vec<Mention<'a>> {
    let mut mentions: Vec<_> = attachments
        .iter()
        .flat_map(|attachment| {
            text.match_indices(&attachment.mention)
                .map(move |(offset, value)| Mention {
                    range: offset..offset + value.len(),
                    attachment,
                })
        })
        .collect();
    mentions.sort_by_key(|mention| mention.range.start);
    let mut end = 0;
    mentions.retain(|mention| {
        if mention.range.start < end {
            return false;
        }
        end = mention.range.end;
        true
    });
    mentions
}

pub(super) fn prompt_content(
    text: &str,
    attachments: &[Attachment],
    capabilities: &acp::PromptCapabilities,
) -> Vec<acp::ContentBlock> {
    let mut blocks = Vec::new();
    let mut start = 0;
    for mention in mentions(text, attachments) {
        let prefix = &text[start..mention.range.start];
        if !prefix.trim().is_empty() {
            blocks.push(prefix.to_owned().into());
        }
        blocks.push(mention.attachment.for_prompt(capabilities));
        start = mention.range.end;
    }
    if !text[start..].trim().is_empty() {
        blocks.push(text[start..].to_owned().into());
    }
    blocks
}

#[cfg(test)]
mod tests;

fn compact_label(name: &str) -> String {
    const MAX_LABEL_CHARS: usize = 24;
    let name = name.replace(['\r', '\n', '\t'], " ");
    if name.chars().count() <= MAX_LABEL_CHARS {
        return name;
    }
    format!(
        "{}…",
        name.chars().take(MAX_LABEL_CHARS - 1).collect::<String>()
    )
}

// Reserve the chip's icon, label and padding in the editor's text layout.
// Word joiners keep the atomic attachment together at a soft wrap.
fn marker(label: &str) -> String {
    let text = format!("⟦□ {}⟧", label.replace(' ', " "));
    text.chars()
        .map(|ch| ch.to_string())
        .collect::<Vec<_>>()
        .join("\u{2060}")
}
