use super::*;
use crate::attachment::{MAX_TOTAL_BYTES, mentions};
use gpui_kit::{ClipboardEntry, EntityInputHandler, PathPromptOptions};
use unicode_segmentation::UnicodeSegmentation;
mod chips;

impl Workspace {
    pub(super) fn prompt_capabilities(&self) -> basr::acp::PromptCapabilities {
        self.current()
            .and_then(|session| session.initialization.as_ref())
            .map(|init| init.agent_capabilities.prompt_capabilities.clone())
            .unwrap_or_default()
    }

    pub(super) fn attachment_error(&self, cx: &App) -> Option<String> {
        let capabilities = self.prompt_capabilities();
        let references = mentions(&self.composer.read(cx).value(), &self.attachments);
        if references
            .iter()
            .map(|item| item.attachment.bytes)
            .sum::<usize>()
            > MAX_TOTAL_BYTES
        {
            return Some("附件总大小不能超过 16 MiB".into());
        }
        references
            .iter()
            .find(|item| !item.attachment.supported(&capabilities))
            .map(|item| format!("当前 Agent 不支持此附件类型：{}", item.attachment.name))
    }

    fn choose_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active else {
            return;
        };
        if !self.loading_attachments.insert(session) {
            return;
        }
        self.error = None;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("添加文件或图片".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = async {
                let paths = paths.await??.unwrap_or_default();
                cx.background_executor()
                    .spawn(async move { load_paths(paths) })
                    .await
            }
            .await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.loading_attachments.remove(&session);
                if this.active == Some(session) {
                    this.focus_input(window, cx);
                }
                match result {
                    Ok(attachments) => this.add_attachments(session, attachments, window, cx),
                    Err(error) if this.active == Some(session) => {
                        this.error = Some(format!("添加附件失败：{error:#}"))
                    }
                    Err(_) => {}
                }
                if this.compact {
                    this.resize_window(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn drop_attachments(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.active else {
            return;
        };
        if self.sending(session) || !self.loading_attachments.insert(session) {
            return;
        }
        self.error = None;
        cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { load_paths(paths) })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.loading_attachments.remove(&session);
                match result {
                    Ok(attachments) => this.add_attachments(session, attachments, window, cx),
                    Err(error) if this.active == Some(session) => {
                        this.error = Some(format!("添加附件失败：{error:#}"))
                    }
                    Err(_) => {}
                }
                if this.compact {
                    this.resize_window(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn add_attachments(
        &mut self,
        session: SessionId,
        mut attachments: Vec<Attachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if attachments.is_empty() || !self.sessions.iter().any(|item| item.id == session) {
            return;
        }
        if self.active == Some(session) {
            self.focus_input(window, cx);
        }
        let capabilities = self
            .sessions
            .iter()
            .find(|item| item.id == session)
            .and_then(|session| session.initialization.as_ref())
            .map(|init| init.agent_capabilities.prompt_capabilities.clone())
            .unwrap_or_default();
        if let Some(attachment) = attachments
            .iter()
            .find(|attachment| !attachment.supported(&capabilities))
        {
            if self.active == Some(session) {
                self.error = Some(format!("当前 Agent 不支持此附件类型：{}", attachment.name));
            }
            cx.notify();
            return;
        }
        let text = if self.active == Some(session) {
            self.composer.read(cx).value().to_string()
        } else {
            self.drafts
                .get(&session)
                .map(|draft| draft.text.clone())
                .unwrap_or_default()
        };
        let target = if self.active == Some(session) {
            &mut self.attachments
        } else {
            &mut self.drafts.entry(session).or_default().attachments
        };
        // Retain removed payloads for undo, but count only references that will be sent.
        let total = mentions(&text, target)
            .iter()
            .map(|item| item.attachment.bytes)
            .sum::<usize>()
            + attachments.iter().map(|item| item.bytes).sum::<usize>();
        if total > MAX_TOTAL_BYTES {
            if self.active == Some(session) {
                self.error = Some("附件总大小不能超过 16 MiB".into());
            }
            cx.notify();
            return;
        }
        let mut inserted = String::new();
        for mut attachment in attachments.drain(..) {
            let mut suffix = 2;
            let label = attachment.label.clone();
            while target.iter().any(|item| item.mention == attachment.mention) {
                attachment.set_label(format!("{label} {suffix}"));
                suffix += 1;
            }
            inserted.push_str(&attachment.mention);
            inserted.push(' ');
            target.push(attachment);
        }
        if self.active == Some(session) {
            self.composer.update(cx, |input, cx| {
                input.replace(inserted, window, cx);
            });
            self.sync_mentions(cx);
            self.error = self.attachment_error(cx);
        } else {
            self.drafts
                .entry(session)
                .or_default()
                .text
                .push_str(&inserted);
        }
        cx.notify();
    }

    pub(super) fn sync_mentions(&mut self, cx: &mut Context<Self>) {
        let selected = self.composer.read(cx).selected_range();
        let mut snapped = selected.clone();
        for mention in mentions(&self.composer.read(cx).value(), &self.attachments) {
            if selected.is_empty()
                && mention.range.start < selected.start
                && selected.start < mention.range.end
            {
                let position = if selected.start >= self.mention_selection.start {
                    mention.range.end
                } else {
                    mention.range.start
                };
                snapped = position..position;
            } else if !selected.is_empty()
                && selected.start < mention.range.end
                && selected.end > mention.range.start
            {
                snapped.start = snapped.start.min(mention.range.start);
                snapped.end = snapped.end.max(mention.range.end);
            }
        }
        self.mention_selection = snapped.clone();
        if snapped != selected {
            self.composer
                .update(cx, |input, cx| input.set_selected_range(snapped, cx));
        }
        let style = gpui_kit::HighlightStyle {
            fade_out: Some(1.),
            ..Default::default()
        };
        let decorations = mentions(&self.composer.read(cx).value(), &self.attachments)
            .into_iter()
            .map(|mention| TextDecoration::new(mention.range, style))
            .collect::<Vec<_>>();
        if self.decorated_mentions != decorations
            || self.mention_decorations.get_ranges(cx)
                != decorations
                    .iter()
                    .map(|item| item.range.clone())
                    .collect::<Vec<_>>()
        {
            self.mention_decorations.set(decorations.clone(), cx);
            self.decorated_mentions = decorations;
        }
    }

    pub(super) fn cross_mention(&mut self, backward: bool, cx: &mut Context<Self>) {
        let input = self.composer.read(cx);
        let selected = input.selected_range();
        if selected.is_empty()
            && let Some(mention) = mentions(&input.value(), &self.attachments)
                .into_iter()
                .find(|mention| {
                    if backward {
                        mention.range.start < selected.start && selected.start <= mention.range.end
                    } else {
                        mention.range.start <= selected.start && selected.start < mention.range.end
                    }
                })
        {
            let target = if backward {
                mention.range.start
            } else {
                mention.range.end
            };
            self.mention_selection = target..target;
            self.composer
                .update(cx, |input, cx| input.set_selected_range(target..target, cx));
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn delete_word_preserving_mentions(
        &mut self,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some_and(|id| self.sending(id)) {
            cx.stop_propagation();
            return;
        }
        let input = self.composer.read(cx);
        if self.attachments.is_empty() {
            cx.propagate();
            return;
        }
        let mut selected = input.selected_range();
        if selected.is_empty() {
            let value = input.value();
            let Some((before, after)) = value.split_at_checked(selected.start) else {
                cx.propagate();
                return;
            };
            // Use the editor's Unicode word-boundary library, without temporarily
            // changing the selection (which would alter native undo semantics).
            if backward {
                selected.start = before
                    .split_word_bound_indices()
                    .rfind(|(_, word)| !word.trim().is_empty())
                    .map_or(0, |(start, _)| start);
            } else {
                selected.end += after
                    .split_word_bound_indices()
                    .find(|(_, word)| !word.trim().is_empty())
                    .map_or(after.len(), |(start, word)| start + word.len());
            }
        }
        self.delete_mention_range(selected, backward, window, cx);
    }

    pub(super) fn delete_mention(
        &mut self,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.composer.read(cx).selected_range();
        self.delete_mention_range(selected, backward, window, cx);
    }

    fn delete_mention_range(
        &mut self,
        selected: std::ops::Range<usize>,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some_and(|id| self.sending(id)) {
            cx.stop_propagation();
            return;
        }
        let input = self.composer.read(cx);
        let value = input.value();
        let mut range = selected.clone();
        let mut matched = false;
        for mention in mentions(&value, &self.attachments) {
            let intersects = if selected.is_empty() {
                if backward {
                    mention.range.start < selected.start && selected.start <= mention.range.end
                } else {
                    mention.range.start <= selected.start && selected.start < mention.range.end
                }
            } else {
                selected.start < mention.range.end && selected.end > mention.range.start
            };
            if intersects {
                range.start = range.start.min(mention.range.start);
                range.end = range.end.max(mention.range.end);
                matched = true;
            }
        }
        if !matched {
            cx.propagate();
            return;
        }
        self.composer.update(cx, |input, cx| {
            let utf16 = input.text().offset_to_offset_utf16(range.start)
                ..input.text().offset_to_offset_utf16(range.end);
            input.replace_text_in_range(Some(utf16), "", window, cx);
        });
        self.sync_mentions(cx);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn paste_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some_and(|id| self.sending(id)) {
            cx.stop_propagation();
            return;
        }
        let Some(clipboard) = cx.read_from_clipboard() else {
            cx.propagate();
            return;
        };
        let images: Vec<_> = clipboard
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                _ => None,
            })
            .collect();
        if images.is_empty() {
            let paths: Vec<_> = clipboard
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                    _ => None,
                })
                .flatten()
                .cloned()
                .collect();
            if !paths.is_empty() {
                cx.stop_propagation();
                self.drop_attachments(paths, window, cx);
                return;
            }
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        let text = clipboard.text();
        self.error = None;
        if !self.prompt_capabilities().image {
            self.error = Some("当前 Agent 不支持图片输入".into());
        } else if let Some(session) = self.active {
            let result: anyhow::Result<Vec<_>> =
                images.into_iter().map(Attachment::image).collect();
            match result {
                Ok(attachments) => self.add_attachments(session, attachments, window, cx),
                Err(error) => self.error = Some(format!("粘贴图片失败：{error:#}")),
            }
        }
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            let error = self.error.clone();
            self.composer
                .update(cx, |input, cx| input.replace(text, window, cx));
            self.error = error;
            self.sync_mentions(cx);
            self.save_draft(cx);
        }
        cx.notify();
    }

    pub(super) fn attachment_button(&self, cx: &mut Context<Self>) -> Button {
        let busy = self
            .active
            .is_some_and(|id| self.loading_attachments.contains(&id) || self.sending(id));
        Button::new("add-attachment")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .tooltip("在光标处插入文件或图片 · Ctrl V 粘贴图片")
            .disabled(self.active.is_none() || busy)
            .on_click(cx.listener(|this, _, window, cx| this.choose_attachments(window, cx)))
    }
}

fn load_paths(paths: Vec<PathBuf>) -> anyhow::Result<Vec<Attachment>> {
    let mut attachments = Vec::new();
    let mut total = 0;
    for path in paths {
        let attachment = Attachment::load(&path)?;
        total += attachment.bytes;
        if total > MAX_TOTAL_BYTES {
            anyhow::bail!("附件总大小不能超过 16 MiB");
        }
        attachments.push(attachment);
    }
    Ok(attachments)
}
