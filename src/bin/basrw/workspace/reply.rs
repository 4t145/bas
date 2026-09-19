//! Completed replies remain discoverable while the compact conversation is folded.

use std::{cell::Cell, rc::Rc, time::Duration};

use basr::model::Message;
use gpui_kit::{
    AnyWindowHandle, Bounds, WeakEntity, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions, canvas, point,
    popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions},
};

use super::*;

const PREVIEW_WIDTH: f32 = 600.;
const PREVIEW_HEIGHT: f32 = 360.;
const IMAGE_PREVIEW_WIDTH: f32 = 360.;
const IMAGE_PREVIEW_HEIGHT: f32 = 240.;
const HOVER_DELAY: Duration = Duration::from_millis(350);
const CLOSE_DELAY: Duration = Duration::from_millis(200);
const SUMMARY_CHARACTERS: usize = 120;

pub(super) struct Preview {
    pub window: Option<AnyWindowHandle>,
    pub content: Entity<ReplyPreview>,
    pub size: Size<Pixels>,
    key: PreviewKey,
    _closed: Subscription,
}

pub(super) struct ReplyPreview {
    owner: WeakEntity<Workspace>,
    parent: AnyWindowHandle,
    message: Message,
    key: PreviewKey,
    documents: Vec<Option<Entity<gpui_kit::component::text::TextViewState>>>,
    _changed: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewKey {
    Reply(u64),
    Attachment(uuid::Uuid),
}

impl Workspace {
    fn completed_reply(&self) -> Option<(&Message, crate::conversation::CompletedReply)> {
        let conversation = self.conversation.as_ref()?;
        if conversation.session.status != SessionStatus::Idle {
            return None;
        }
        let reply = conversation.completed_reply?;
        let Item::Message { message, .. } = conversation.items.get(reply.index)? else {
            return None;
        };
        Some((message, reply))
    }

    fn reply_is_unread(&self, cursor: u64) -> bool {
        self.active
            .is_some_and(|session| self.seen_replies.get(&session).copied().unwrap_or(0) < cursor)
    }

    pub(super) fn has_unread_reply(&self) -> bool {
        self.completed_reply()
            .is_some_and(|(_, reply)| self.reply_is_unread(reply.cursor))
    }

    pub(super) fn sync_reply_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.completed_reply().map(|(_, reply)| reply);
        let completed = reply.map(|reply| reply.cursor);
        if self
            .reply_preview
            .as_ref()
            .is_some_and(|preview| match preview.key {
                PreviewKey::Reply(cursor) => {
                    Some(cursor) != completed || !self.compact || self.output_open
                }
                PreviewKey::Attachment(id) => {
                    !crate::attachment::mentions(&self.composer.read(cx).value(), &self.attachments)
                        .iter()
                        .any(|mention| mention.attachment.id == id)
                }
            })
        {
            self.close_reply_preview(window, cx);
        }
        if window.is_window_active()
            && (!self.compact || self.output_open)
            && (self.list.is_following_tail()
                || reply.is_some_and(|reply| {
                    self.list
                        .bounds_for_item(reply.index)
                        .is_some_and(|bounds| {
                            let viewport = self.list.viewport_bounds();
                            viewport.size.height > px(0.)
                                && bounds.bottom() <= viewport.bottom()
                                && bounds.bottom() > viewport.top()
                        })
                }))
            && let Some(cursor) = completed
            && let Some(session) = self.active
        {
            self.seen_replies.insert(session, cursor);
        }
    }

    pub(super) fn render_reply_summary(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (message, reply) = self.completed_reply()?;
        let text = message.plain_text();
        let prefix: String = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .chars()
            .take(SUMMARY_CHARACTERS)
            .collect();
        let summary = prefix.split_whitespace().collect::<Vec<_>>().join(" ");
        let summary = if summary.is_empty() {
            "回复已完成".to_owned()
        } else {
            summary
        };
        let unread = self.reply_is_unread(reply.cursor);
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        Some(
            div()
                .id("completed-reply")
                .test_support()
                .relative()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .cursor_pointer()
                .text_color(if unread {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(div().flex_1().min_w_0().truncate().child(summary))
                .child(
                    canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    if !this.output_open {
                        this.toggle_output(window, cx);
                    }
                    this.list.set_follow_mode(FollowMode::Tail);
                    cx.notify();
                }))
                .on_hover(cx.listener(move |this, hovered, window, cx| {
                    this.hover_reply(*hovered, bounds.get(), window, cx);
                }))
                .into_any_element(),
        )
    }

    pub(super) fn hover_reply(
        &mut self,
        hovered: bool,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reply_hover = None;
        if hovered && (self.reply_preview.is_some() || self.output_open || self.menu.is_some()) {
            return;
        }
        self.reply_hover = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(if hovered { HOVER_DELAY } else { CLOSE_DELAY })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if hovered {
                    this.open_reply_preview(bounds, window, cx);
                } else {
                    this.close_reply_preview(window, cx);
                }
            });
        }));
    }

    pub(super) fn open_reply_preview(
        &mut self,
        trigger: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.compact || self.output_open || self.menu.is_some() || self.reply_preview.is_some()
        {
            return;
        }
        let Some((message, reply)) = self.completed_reply() else {
            return;
        };
        let message = message.clone();
        self.open_content_preview(
            message,
            PreviewKey::Reply(reply.cursor),
            trigger,
            window,
            cx,
        );
    }

    pub(super) fn hover_attachment(
        &mut self,
        id: uuid::Uuid,
        hovered: bool,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reply_hover = None;
        let key = PreviewKey::Attachment(id);
        if hovered
            && self
                .reply_preview
                .as_ref()
                .is_some_and(|preview| preview.key == key)
        {
            return;
        }
        self.reply_hover = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(if hovered { HOVER_DELAY } else { CLOSE_DELAY })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !hovered {
                    this.close_reply_preview(window, cx);
                    return;
                }
                if this.menu.is_some() {
                    return;
                }
                let Some(attachment) = this
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == id)
                else {
                    return;
                };
                let message = Message::new(
                    basr::model::Role::User,
                    vec![attachment.content.as_ref().clone()],
                );
                this.close_reply_preview(window, cx);
                this.open_content_preview(message, key, bounds, window, cx);
            });
        }));
    }

    fn open_content_preview(
        &mut self,
        message: Message,
        key: PreviewKey,
        trigger: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen = window
            .display(cx)
            .map(|display| display.bounds().size)
            .unwrap_or(window.bounds().size);
        let (width, height) = match key {
            PreviewKey::Reply(_) => (PREVIEW_WIDTH, PREVIEW_HEIGHT),
            PreviewKey::Attachment(_) => (IMAGE_PREVIEW_WIDTH, IMAGE_PREVIEW_HEIGHT),
        };
        let preview_size = size(
            px(width).min(screen.width * 0.8),
            px(height).min(screen.height * 0.6),
        );
        let owner = cx.entity();
        let parent = window.window_handle();
        let content = cx.new(|cx| {
            let documents: Vec<_> = message
                .content
                .iter()
                .map(|block| {
                    chat::block_source(block).map(|source| {
                        cx.new(|cx| gpui_kit::component::text::TextViewState::markdown(&source, cx))
                    })
                })
                .collect();
            let changed = documents
                .iter()
                .flatten()
                .map(|state| cx.observe(state, |_, _, cx| cx.notify()))
                .collect();
            ReplyPreview {
                owner: owner.downgrade(),
                parent,
                message,
                key,
                documents,
                _changed: changed,
            }
        });
        let preview = content.clone();
        let native = match cx.open_window(
            popup_options(parent, trigger, preview_size),
            |window, cx| {
                cx.new(|cx| {
                    Root::new(preview, window, cx)
                        .bordered(false)
                        .window_shadow_size(px(0.))
                })
            },
        ) {
            Ok(handle) => Some(handle.into()),
            Err(error) => {
                tracing::debug!(%error, "using an inline reply preview on this platform");
                None
            }
        };
        let owner = owner.downgrade();
        let closed = cx.on_window_closed(move |cx, id| {
            if native.is_some_and(|popup: AnyWindowHandle| popup.window_id() == id) {
                let _ = owner.update(cx, |this, cx| {
                    if this
                        .reply_preview
                        .as_ref()
                        .is_some_and(|preview| preview.window == native)
                    {
                        this.reply_preview = None;
                        cx.notify();
                    }
                });
            }
        });
        self.reply_preview = Some(Preview {
            window: native,
            content,
            size: preview_size,
            key,
            _closed: closed,
        });
        if native.is_none() {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    pub(super) fn close_reply_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_hover = None;
        let Some(preview) = self.reply_preview.take() else {
            return;
        };
        if let Some(popup) = preview.window {
            // Closing may originate from the preview's own hover callback.
            cx.defer(move |cx| {
                let _ = popup.update(cx, |_, popup, _| popup.remove_window());
            });
        } else {
            self.resize_window(window, cx);
        }
        cx.notify();
    }
}

impl Render for ReplyPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocks = self
            .message
            .content
            .iter()
            .zip(&self.documents)
            .enumerate()
            .map(|(index, (block, document))| {
                if matches!(self.key, PreviewKey::Attachment(_))
                    && let basr::acp::ContentBlock::Image(image) = block
                {
                    return div()
                        .w_full()
                        .h(px(IMAGE_PREVIEW_HEIGHT - 26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(chat::decode_image(image).map(|image| {
                            gpui_kit::img(image)
                                .size_full()
                                .object_fit(gpui_kit::ObjectFit::Contain)
                        }))
                        .into_any_element();
                }
                chat::render_content(
                    format!("preview-{}-{index}", self.message.id),
                    block,
                    document.as_ref().map(|state| markdown::view(state, cx)),
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let owner = self.owner.clone();
        let parent = self.parent;
        let key = self.key;
        div()
            .id("reply-preview")
            .test_support()
            .size_full()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(theme::content_background(cx))
            .text_color(cx.theme().foreground)
            .refine_style(&theme::font_style(cx))
            .text_size(theme::content_font_size(cx))
            .overflow_hidden()
            .child(
                div()
                    .id("reply-preview-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(
                        blocks
                            .into_iter()
                            .map(|block| div().flex_shrink_0().child(block)),
                    ),
            )
            .on_hover(move |hovered, window, cx| {
                menus::in_parent(parent, window, cx, |window, cx| {
                    let _ = owner.update(cx, |this, cx| match key {
                        PreviewKey::Reply(_) => {
                            this.hover_reply(*hovered, Bounds::default(), window, cx)
                        }
                        PreviewKey::Attachment(id) => {
                            this.hover_attachment(id, *hovered, Bounds::default(), window, cx)
                        }
                    });
                });
            })
    }
}

fn popup_options(
    parent: AnyWindowHandle,
    trigger: Bounds<Pixels>,
    preview_size: Size<Pixels>,
) -> WindowOptions {
    WindowOptions {
        kind: WindowKind::AnchoredPopup(PopupOptions {
            parent,
            anchor_rect: trigger,
            anchor: PopupAnchor::TopLeft,
            gravity: PopupGravity::TopRight,
            constraint_adjustment: PopupConstraintAdjustment::FLIP_Y
                | PopupConstraintAdjustment::SLIDE_X
                | PopupConstraintAdjustment::SLIDE_Y,
            offset: point(px(0.), px(0.)),
            grab: false,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            preview_size,
        ))),
        titlebar: None,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.basr.ReplyPreview".into()),
        ..Default::default()
    }
}
