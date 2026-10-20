//! Completed replies remain discoverable while the compact conversation is folded.

use std::{cell::Cell, rc::Rc, time::Duration};

use bas::model::Message;
use gpui_kit::{
    AnyView, AnyWindowHandle, Bounds, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, canvas, point,
    popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions},
};

use super::*;

const PREVIEW_WIDTH: f32 = 600.;
const PREVIEW_HEIGHT: f32 = 360.;
const IMAGE_PREVIEW_WIDTH: f32 = 360.;
const IMAGE_PREVIEW_HEIGHT: f32 = 240.;
pub(super) const HOVER_DELAY: Duration = Duration::from_millis(350);
pub(super) const CLOSE_DELAY: Duration = Duration::from_millis(200);
const SUMMARY_CHARACTERS: usize = 120;

pub(super) struct Preview {
    pub window: Option<AnyWindowHandle>,
    pub content: AnyView,
    pub size: Size<Pixels>,
    pub(super) key: PreviewKey,
    trigger: Bounds<Pixels>,
    _closed: Subscription,
}

pub(super) struct ReplyPreview {
    owner: WeakEntity<Workspace>,
    parent: AnyWindowHandle,
    message: Message,
    key: PreviewKey,
    documents: Vec<Option<Entity<gpui_kit::component::text::TextViewState>>>,
    images: Vec<Entity<images::Cache>>,
    audio: Entity<crate::audio::Player>,
    links: links::Context,
    _changed: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewKey {
    Reply(u64),
    Attachment(uuid::Uuid),
    Activity,
}

impl Workspace {
    fn completed_reply(&self) -> Option<(&Message, crate::conversation::ReplyPosition)> {
        let conversation = self.selected.timeline.conversation.as_ref()?;
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
        self.selected.id.is_some_and(|session| {
            self.sessions
                .seen_replies
                .get(&session)
                .copied()
                .unwrap_or(0)
                < cursor
        })
    }

    pub(super) fn has_unread_reply(&self) -> bool {
        self.completed_reply()
            .is_some_and(|(_, reply)| self.reply_is_unread(reply.cursor))
    }

    pub(super) fn sync_reply_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.completed_reply().map(|(_, reply)| reply);
        let completed = reply.map(|reply| reply.cursor);
        if self
            .window
            .reply_preview
            .as_ref()
            .is_some_and(|preview| match preview.key {
                PreviewKey::Reply(cursor) => {
                    Some(cursor) != completed || !self.window.compact || self.window.output_open
                }
                PreviewKey::Attachment(id) => !crate::attachment::mentions(
                    &self.composer.input.read(cx).value(),
                    &self.composer.attachments,
                )
                .iter()
                .any(|mention| mention.attachment.id == id),
                PreviewKey::Activity => false,
            })
        {
            self.close_reply_preview(window, cx);
        }
        let visible_reply = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.last_reply);
        if self.window.presented
            && window.is_window_active()
            && (!self.window.compact || self.window.output_open)
            && (self.selected.timeline.list.is_following_tail()
                || visible_reply.is_some_and(|reply| {
                    self.selected
                        .timeline
                        .conversation
                        .as_ref()
                        .and_then(|conversation| conversation.row_for_item(reply.index))
                        .and_then(|index| self.selected.timeline.list.bounds_for_item(index))
                        .is_some_and(|bounds| {
                            let viewport = self.selected.timeline.list.viewport_bounds();
                            viewport.size.height > px(0.)
                                && bounds.bottom() <= viewport.bottom()
                                && bounds.bottom() > viewport.top()
                        })
                }))
            && let Some(reply) = visible_reply
            && let Some(session) = self.selected.id
        {
            self.see_reply(session, reply.cursor, cx);
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
            crate::i18n::tr!("response-completed").to_owned()
        } else {
            summary
        };
        let unread = self.reply_is_unread(reply.cursor);
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        Some(gds_uim::el! {
            #div {
                id: "completed-reply"; test_support; relative; flex; flex_1;
                min_w_0; items_center; gap_1p5; cursor_pointer;
                text_color: $(if unread { cx.theme().foreground } else { cx.theme().muted_foreground });
                on_mouse_down: $gpui_kit::MouseButton::Left, $Self::on_reply_mouse_down;
                on_click: $cx.listener(Self::on_reply_open);
                on_hover: $cx.listener(move |this, hovered, window, cx| {
                    this.hover_reply(*hovered, bounds.get(), window, cx);
                });
            } [
                #div { flex_1; min_w_0; truncate; } [ $summary ],
                #canvas($move |bounds, _, _| measured.set(bounds), $|_, _, _, _| {}) {
                    absolute; size_full;
                }
            ]
        }.into_any_element())
    }

    fn on_reply_mouse_down(_: &gpui_kit::MouseDownEvent, _: &mut Window, cx: &mut App) {
        cx.stop_propagation();
    }

    fn on_reply_open(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.window.output_open {
            self.toggle_output(window, cx);
        }
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub(super) fn hover_reply(
        &mut self,
        hovered: bool,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window.reply_hover = None;
        if hovered
            && (self.window.reply_preview.is_some()
                || self.window.output_open
                || self.window.menu.is_some())
        {
            return;
        }
        self.window.reply_hover = Some(cx.spawn_in(window, async move |this, cx| {
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
        if !self.window.compact
            || self.window.output_open
            || self.window.menu.is_some()
            || self.window.reply_preview.is_some()
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
        self.window.reply_hover = None;
        let key = PreviewKey::Attachment(id);
        if hovered
            && self
                .window
                .reply_preview
                .as_ref()
                .is_some_and(|preview| preview.key == key)
        {
            return;
        }
        self.window.reply_hover = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(if hovered { HOVER_DELAY } else { CLOSE_DELAY })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !hovered {
                    this.close_reply_preview(window, cx);
                    return;
                }
                if this.window.menu.is_some() {
                    return;
                }
                let Some(attachment) = this
                    .composer
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == id)
                else {
                    return;
                };
                let message = Message::new(
                    bas::model::Role::User,
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
        let owner = cx.entity();
        let parent = window.window_handle();
        let directory = self
            .current()
            .and_then(|session| session.cwd.as_deref().map(navigation::groups::normalize))
            .or_else(|| self.navigation.selected_directory.clone());
        let content = cx.new(|cx| {
            let documents: Vec<_> = message
                .content
                .iter()
                .map(|block| {
                    chat::content::block_source(block).map(|source| {
                        cx.new(|cx| gpui_kit::component::text::TextViewState::markdown(&source, cx))
                    })
                })
                .collect();
            let images: Vec<_> = message
                .content
                .iter()
                .map(|_| images::Cache::new(directory.clone(), cx))
                .collect();
            let mut changed: Vec<_> = documents
                .iter()
                .flatten()
                .map(|state| cx.observe(state, ReplyPreview::on_content_changed))
                .collect();
            changed.extend(
                images
                    .iter()
                    .map(|cache| cx.observe(cache, ReplyPreview::on_content_changed)),
            );
            changed.push(cx.observe(&self.audio, ReplyPreview::on_content_changed));
            ReplyPreview {
                owner: owner.downgrade(),
                parent,
                message,
                key,
                audio: self.audio.clone(),
                links: self.link_context(),
                documents,
                images,
                _changed: changed,
            }
        });
        self.open_preview(content.into(), key, trigger, window, cx);
    }

    pub(super) fn open_preview(
        &mut self,
        content: AnyView,
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
            PreviewKey::Activity => (activity::PANEL_WIDTH, activity::PANEL_HEIGHT),
        };
        let preview_size = size(
            px(width).min(screen.width * 0.8),
            px(height).min(screen.height * 0.6),
        );
        let owner = cx.entity();
        let parent = window.window_handle();
        let preview = content.clone();
        let native = match cx.open_window(
            popup_options(parent, trigger, preview_size),
            |window, cx| {
                cx.new(|cx| {
                    gds_uim::el! { #theme::window_root(preview, window, cx) {
                        bordered: false; window_shadow_size: 0.0.px;
                    } }
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
                        .window
                        .reply_preview
                        .as_ref()
                        .is_some_and(|preview| preview.window == native)
                    {
                        this.window.reply_preview = None;
                        cx.notify();
                    }
                });
            }
        });
        self.window.reply_preview = Some(Preview {
            window: native,
            content,
            size: preview_size,
            key,
            trigger,
            _closed: closed,
        });
        if native.is_none() {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    pub(super) fn inline_preview(&self) -> Option<AnyElement> {
        let preview = self
            .window
            .reply_preview
            .as_ref()
            .filter(|preview| preview.window.is_none() && !self.window.compact)?;
        Some(gpui_kit::deferred(gds_uim::el! {
            #gpui_kit::anchored {
                position: $preview.trigger.bottom_left();
                snap_to_window_with_margin: 8.0.px;
            } [
                #div { w: $preview.size.width; h: $preview.size.height; } [$preview.content.clone()],
            ]
        }).with_priority(gpui_kit::base::POPUP_PRIORITY).into_any_element())
    }

    pub(super) fn close_reply_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window.reply_hover = None;
        self.window.activity_hover = None;
        let Some(preview) = self.window.reply_preview.take() else {
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

impl ReplyPreview {
    fn on_content_changed<T: 'static>(&mut self, _: Entity<T>, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn on_hover(&mut self, hovered: &bool, window: &mut Window, cx: &mut Context<Self>) {
        menus::in_parent(self.parent, window, cx, |window, cx| {
            let _ = self.owner.update(cx, |this, cx| match self.key {
                PreviewKey::Reply(_) => this.hover_reply(*hovered, Bounds::default(), window, cx),
                PreviewKey::Attachment(id) => {
                    this.hover_attachment(id, *hovered, Bounds::default(), window, cx)
                }
                PreviewKey::Activity => {
                    this.hover_activity(*hovered, Bounds::default(), window, cx)
                }
            });
        });
    }
}

impl Render for ReplyPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocks = self
            .message
            .content
            .iter()
            .zip(self.documents.iter().zip(&self.images))
            .enumerate()
            .map(|(index, (block, (document, images)))| {
                let id = format!("preview-{}-{index}", self.message.id);
                if let bas::acp::ContentBlock::Audio(audio) = block {
                    let playback_id = match self.key {
                        PreviewKey::Reply(_) => {
                            chat::content::message_block_id(&self.message, index)
                        }
                        PreviewKey::Attachment(attachment) => {
                            format!("attachment-{attachment}-{index}")
                        }
                        PreviewKey::Activity => id.clone(),
                    };
                    return crate::audio::content(&id, &playback_id, audio, &self.audio, cx);
                }
                if matches!(self.key, PreviewKey::Attachment(_))
                    && let bas::acp::ContentBlock::Image(image) = block
                {
                    return gds_uim::el! { #div {
                        w_full; h: $px(IMAGE_PREVIEW_HEIGHT - 26.);
                        flex; items_center; justify_center;
                    } [ $images::acp_image(&id, image, images, true, cx) ] }
                    .into_any_element();
                }
                chat::content::render_content(
                    id.clone(),
                    block,
                    document.as_ref().map(|state| {
                        images::wrap(
                            &id,
                            images,
                            state,
                            markdown::view(state, cx)
                                .links(self.links.clone())
                                .into_any_element(),
                            cx,
                        )
                    }),
                    images,
                    &self.audio,
                    &self.links,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let blocks = blocks.into_iter().map(|block| {
            gds_uim::el! {
                #div { flex_shrink_0; } [ $block ]
            }
        });
        gds_uim::el! {
            #div {
                id: "reply-preview"; test_support; size_full; rounded_md; border_1;
                border_color: $cx.theme().border; bg: $theme::content_background(cx);
                text_color: $cx.theme().foreground; refine_style: $&theme::font_style(cx);
                text_size: $theme::content_font_size(cx); overflow_hidden;
                on_hover: $cx.listener(Self::on_hover);
            } [
                #div {
                    id: "reply-preview-scroll"; size_full; overflow_y_scroll;
                    p_3; flex; flex_col; gap_2;
                } [ ..blocks ]
            ]
        }
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
        app_id: Some("org.bas.ReplyPreview".into()),
        ..Default::default()
    }
}
