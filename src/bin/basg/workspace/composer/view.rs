use gpui_kit::ClickEvent;

use super::*;

const COMPOSER_TEXT_INSET: f32 =
    COMPOSER_CONTROL_INSET + (layout::ICON_BUTTON_SIZE - agent::TOOLBAR_AGENT_ICON_SIZE) / 2.;
const COMPOSER_CONTROL_GAP: f32 = 4.;
// The pinned editor reserves 10px each for its gutter and caret at the right edge.
const EDITOR_WRAP_INSET: f32 = 20.;

impl Workspace {
    pub(in crate::workspace) fn composer_expansion_button(&self, cx: &mut Context<Self>) -> Button {
        el! { (
            "expand-message-editor",
            @if (self.composer.expanded)
                IconName::Minimize2
            @else
                IconName::Maximize2
            @end,
            cx,
        ).message_actions::action_button {
            hoverable_tooltip: $shortcuts::tooltip(if self.composer.expanded {
                tr!("composer-collapse")
            } else {
                tr!("composer-expand")
            }, ExpandMessageEditor);
            on_click: (cx, Self::on_composer_toggle_expansion).Context::listener;
        } }
    }

    fn on_composer_toggle_expansion(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_composer_expansion(window, cx);
    }

    pub(in crate::workspace) fn composer_fills_thread(&self) -> bool {
        !self.window.compact
            && !self.awaiting_connection()
            && self.composer.expanded
            && !self.selected.awaiting_history()
            && !self.thread_has_user_prompt()
            && !self.working()
            && self.selected.approval.panel.is_empty()
            && self.selected.authentication.panel.is_none()
            && self.selected.elicitations.is_empty()
            && self.selected.id.is_none_or(|id| {
                !self
                    .sessions
                    .outboxes
                    .get(&id)
                    .is_some_and(outbox::SessionOutbox::pending)
            })
    }

    pub(in crate::workspace) fn measure_composer(&mut self, cx: &mut Context<Self>) {
        let input = self.composer.input.read(cx);
        let width = input
            .text_bounds()
            .map(|bounds| (bounds.size.width - px(EDITOR_WRAP_INSET)).max(px(1.)));
        let text = input.text().clone();
        let mut font = gpui_kit::font(cx.theme().font_family.clone());
        font.fallbacks = theme::font_style(cx).text.font_fallbacks;
        self.composer.rows =
            self.composer
                .layout
                .measure(&text, font, theme::content_font_size(cx), width, cx);
    }

    pub(in crate::workspace) fn composer_height(&self, cx: &App) -> f32 {
        let input = self.composer.input.read(cx);
        let rows = input_layout::visible_rows(
            self.composer.rows,
            input.text().lines_len(),
            self.window.compact,
            self.composer.expanded,
        );
        let height = rows as f32
            * (f32::from(theme::content_font_size(cx)) * theme::content_line_height(cx)).ceil()
            + f32::from(gpui_kit::component::Size::Medium.input_py()) * 2.;
        if self.composer.expanded && !self.window.compact {
            height.min(self.composer.available_height.get())
        } else {
            height
        }
    }

    pub(in crate::workspace) fn feedback_height(&self, cx: &App) -> f32 {
        if self.composer_error(cx).is_some() {
            ui::CONTROL
        } else {
            0.
        }
    }

    fn render_composer_feedback(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let message = self.composer_error(cx)?;
        let retry = self.selected.timeline.conversation.is_some()
            && matches!(self.selected.stream, StreamState::Unavailable(_));
        Some(
            el! {
                .div {
                    flex; items_center; min_w_0; px_2;
                    h: ui::CONTROL.px;
                    flex_shrink_0;
                } [
                    .div { flex_1; min_w_0; } [
                        ("composer-feedback", message, $cx.theme().danger).ui::feedback { compact; },
                    ],
                    @if (retry)
                        ("history-resync-retry", IconName::RefreshCw, $self.history_retry_label()).ui::icon_action {
                            flex_shrink_0;
                            disabled: $self.history_retry_disabled();
                            on_click: $cx.listener(|this, _, _, cx| this.retry_history(cx));
                        },
                    @end,
                ]
            }
            .into_any_element(),
        )
    }

    pub(in crate::workspace) fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let fills_thread = self.composer_fills_thread();
        let line_height =
            (f32::from(theme::content_font_size(cx)) * theme::content_line_height(cx)).ceil();
        let expand_top = gpui_kit::component::Size::Medium.input_py()
            + px((line_height - layout::ICON_BUTTON_SIZE) / 2.);
        let composition_locked = self
            .selected
            .id
            .is_some_and(|id| self.composition_locked(id));
        let cancelling = self
            .selected
            .id
            .is_some_and(|id| self.sessions.pending.cancelling.contains(&id));
        let can_send = self.can_send(cx);
        let submit_hint = if self.working() || cancelling {
            tr!("action-stop").into()
        } else {
            self.send_hint(cx)
        };
        let submit = el! {
            @if (self.working() || cancelling)
                "stop-turn".Button::new {
                    ghost;
                    small;
                    icon: IconName::Square;
                    text_color: $cx.theme().danger;
                    @if (self.composer_error(cx).is_some())
                        text_color: $cx.theme().danger;
                    @end;
                    disabled: $(
                    cancelling || self.connection_status() != status::ConnectionStatus::Connected
                    );
                    on_click: (cx, Self::on_composer_stop).Context::listener;
                }
            @else
                "send-message".Button::new {
                    primary;
                    @if (!can_send) ghost; @end;
                    small;
                    icon: IconName::Send;
                    @if (self.composer_error(cx).is_some()) text_color: $cx.theme().danger; @end;
                    disabled: $(!can_send);
                    on_click: (cx, Self::on_composer_send).Context::listener;
                }
            @end
        };
        let panel = el! {
            .div {
                # "composer-panel";
                test_support;
                flex;
                flex_col;
                @if (fills_thread) flex_1; min_h_0; @end;
            }
        }
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, cx| {
            style.bg(cx.theme().primary.opacity(0.12))
        })
        .on_drop(cx.listener(Self::on_composer_drop));
        el! {
            .div {
                flex_shrink_0;
                w_full;
                @if (fills_thread) flex_1; min_h_0; flex; flex_col; @end;
                @if (self.window.compact)
                    px: COMPOSER_SIDE_PADDING.px;
                    pb: COMPOSER_BOTTOM_PADDING.px;
                @else
                    pb: COMPOSER_SIDE_PADDING.px;
                @end;
            } [
                panel {
                    @if (!self.window.compact) border_t_1; border_color: $cx.theme().border; @end;
                } [
                    $self.render_commands(cx),
                    ..$self.render_composer_feedback(cx),
                    $self.render_composer_editor(fills_thread, composition_locked, expand_top, cx),
                    $self.render_composer_toolbar(submit, submit_hint, cx),
                ],
            ]
        }
        .into_any_element()
    }

    fn render_composer_editor(
        &self,
        fills_thread: bool,
        composition_locked: bool,
        expand_top: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Trim the component's measured blank inset while retaining native editing and overlays.
        let inset = -(self.composer.content_left.get() - px(COMPOSER_TEXT_INSET)).max(px(0.));
        let copyable = self.composer.input.read(cx).is_copyable();
        let editable = !(composition_locked || self.composer.paste_pending);
        let editor = el! {
            ($&self.composer.input).Editor::new {
                w_auto; flex_1; min_w_0;
                ml: inset;
                font_family: ~$(cx.theme().font_family);
                text_size: $theme::content_font_size(cx);
                line_height: $gpui_kit::relative(theme::content_line_height(cx));
                appearance: false;
                bordered: false;
                readonly: $(composition_locked || self.composer.paste_pending);
                context_menu: move (menu, _, cx) => (menu, editable, copyable, cx).super::text_menu;
                aria_label: tr!("composer-message");
                @if (fills_thread)
                    h_full;
                @else
                    h: $(self.composer_height(cx)).px;
                @end;
            }
        };
        let editor = ui::input::placeholder(&self.composer.input, editor);
        el! {
            .div {
                # "composer-editor";
                relative;
                key_context: "BasComposer";
                test_support;
                capture_action: (cx, Self::on_composer_expand_action).Context::listener;
                capture_action: (cx, Self::on_composer_send_action).Context::listener;
                capture_action: (cx, Self::capture_composer_enter).Context::listener;
                capture_action: (cx, Self::on_composer_paste_action).Context::listener;
                capture_action: (cx, Self::on_composer_backspace_action).Context::listener;
                capture_action: (cx, Self::on_composer_delete_action).Context::listener;
                capture_action: (cx, Self::on_composer_previous_word_action).Context::listener;
                capture_action: (cx, Self::on_composer_next_word_action).Context::listener;
                capture_action: (cx, Self::on_composer_left_action).Context::listener;
                capture_action: (cx, Self::on_composer_right_action).Context::listener;
                capture_action: (cx, Self::on_composer_up_action).Context::listener;
                capture_action: (cx, Self::on_composer_down_action).Context::listener;
                capture_action: (cx, Self::on_composer_tab_action).Context::listener;
                capture_action: (cx, Self::on_composer_escape_action).Context::listener;
                flex;
                items_start;
                pr: $(COMPOSER_CONTROL_INSET + layout::ICON_BUTTON_SIZE + COMPOSER_CONTROL_GAP).px;
                @if (fills_thread) flex_1; min_h_0; @end;
            } [
                .div {
                    relative; flex; flex_1; min_w_0; overflow_x_hidden;
                    @if (fills_thread) h_full; @end;
                } [editor, $self.render_attachment_chips(cx)],
                .div {
                    absolute;
                    top: $expand_top.max(px(0.));
                    right: COMPOSER_CONTROL_INSET.px;
                } [$self.composer_expansion_button(cx)],
            ]
        }
        .into_any_element()
    }

    fn render_composer_toolbar(
        &self,
        submit: Button,
        submit_hint: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        el! {
            .div {
                # "composer-toolbar";
                test_support; w_full; flex; items_center; gap_1;
                pl: COMPOSER_CONTROL_INSET.px;
                pr: COMPOSER_CONTROL_INSET.px;
                h: COMPOSER_TOOLBAR_HEIGHT.px;
                flex_shrink_0;
            } [
                ..$self.render_usage(cx),
                $(self.attachment_button(cx)) {
                    w: layout::ICON_BUTTON_SIZE.px; h: layout::ICON_BUTTON_SIZE.px; p_0;
                },
                @if (self.supports_directories())
                    $self.directories_button(cx),
                @end,
                .div { flex_1; min_w_0; } [$self.render_config_options(cx)],
                ..$self.render_queue(cx),
                @if (self.working() && !self.composer.input.read(cx).value().trim().is_empty())
                    $self.immediate_send_button(cx),
                @end,
                @if (!self.window.compact && self.selected.authentication.required.is_some())
                    $self.authentication_button(cx),
                @end,
                @if (self.working() && !self.composer.input.read(cx).value().trim().is_empty())
                    (
                        "queue-message-hint",
                        "queue-message".Button::new {
                            ghost; xsmall; icon: IconName::Send;
                            disabled: $(!self.can_send(cx));
                            on_click: (cx, Self::on_composer_send).Context::listener;
                        },
                        $self.send_hint(cx),
                    ).ui::tooltip::bounded,
                @end,
                (
                    "submit-hint",
                    submit {
                        xsmall;
                        w: layout::ICON_BUTTON_SIZE.px;
                        h: layout::ICON_BUTTON_SIZE.px;
                        p_0;
                        flex_shrink_0;
                    },
                    submit_hint,
                ).ui::tooltip::bounded,
            ]
        }
        .into_any_element()
    }

    fn on_composer_stop(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.selected.id
            && self.sessions.healthy(session)
        {
            self.sessions.pending.cancelling.insert(session);
            self.sessions.outboxes.entry(session).or_default().pause();
            self.command(Command::Cancel(session), cx);
            cx.notify();
        }
    }

    fn on_composer_send(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.send_prompt(window, cx);
    }

    fn on_composer_drop(
        &mut self,
        paths: &gpui_kit::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_attachments(paths.paths().to_vec(), window, cx);
    }

    fn on_composer_expand_action(
        &mut self,
        _: &composer::ExpandMessageEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_composer_expansion(window, cx);
        cx.stop_propagation();
    }

    fn on_composer_send_action(
        &mut self,
        _: &composer::SendComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_composer(window, cx);
        cx.stop_propagation();
    }

    fn on_composer_paste_action(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        self.paste_attachment(window, cx);
    }

    fn on_composer_backspace_action(
        &mut self,
        _: &Backspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_mention(true, window, cx);
    }

    fn on_composer_delete_action(
        &mut self,
        _: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_mention(false, window, cx);
    }

    fn on_composer_previous_word_action(
        &mut self,
        _: &DeleteToPreviousWordStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_word_preserving_mentions(true, window, cx);
    }

    fn on_composer_next_word_action(
        &mut self,
        _: &DeleteToNextWordEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_word_preserving_mentions(false, window, cx);
    }

    fn on_composer_left_action(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.cross_mention(true, cx);
    }

    fn on_composer_right_action(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.cross_mention(false, cx);
    }

    fn on_composer_up_action(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.command_action("up", window, cx);
    }

    fn on_composer_down_action(
        &mut self,
        _: &MoveDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_action("down", window, cx);
    }

    fn on_composer_tab_action(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_action("tab", window, cx);
    }

    fn on_composer_escape_action(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_action("escape", window, cx);
    }
}
