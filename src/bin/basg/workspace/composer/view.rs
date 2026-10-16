use gpui_kit::ClickEvent;

use super::*;

const COMPOSER_TEXT_INSET: f32 =
    COMPOSER_CONTROL_INSET + (layout::ICON_BUTTON_SIZE - agent::TOOLBAR_AGENT_ICON_SIZE) / 2.;
const COMPOSER_CONTROL_GAP: f32 = 4.;
// The pinned editor reserves 10px each for its gutter and caret at the right edge.
const EDITOR_WRAP_INSET: f32 = 20.;

impl Workspace {
    pub(in crate::workspace) fn composer_expansion_button(&self, cx: &mut Context<Self>) -> Button {
        gds_uim::el! { #message_actions::action_button(
            "expand-message-editor",
            $(if self.composer.expanded {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            }),
            cx,
        ) {
            tooltip: $(if self.composer.expanded {
                crate::i18n::tr!("composer-collapse")
            } else {
                crate::i18n::tr!("composer-expand")
            });
            on_click: $cx.listener(Self::on_composer_toggle_expansion);
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
            && self.composer.expanded
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
            crate::ui::CONTROL
        } else {
            0.
        }
    }

    fn render_composer_feedback(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let message = self.composer_error(cx)?;
        let detail = message.clone();
        Some(
            gds_uim::el! {
                #div {
                    id: "composer-feedback";
                    test_support; flex; items_center; gap_2; px_2;
                    h: $px(crate::ui::CONTROL);
                    flex_shrink_0;
                    text_size: $crate::ui::SMALL;
                    text_color: $cx.theme().danger;
                } [
                    #div {
                        id: "composer-feedback-message";
                        test_support;
                        role: $gpui_kit::Role::Label;
                        aria_label: $message.clone();
                        flex_1; min_w_0; truncate;
                    } [$message.replace('\n', " ")],
                    $layout::icon_button("composer-feedback-details", IconName::Info) {
                        accessibility_label: $crate::i18n::tr!("feedback-details");
                        tooltip: $crate::i18n::tr!("feedback-details");
                        on_click: $move |_, _, cx| Self::on_composer_feedback_details(&detail, cx);
                    },
                ]
            }
            .into_any_element(),
        )
    }

    fn on_composer_feedback_details(detail: &str, cx: &mut App) {
        if let Err(error) =
            crate::application::open_message(crate::i18n::tr!("feedback-title"), detail, cx)
        {
            tracing::error!(%error, "cannot open operation details");
        }
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
            crate::i18n::tr!("action-stop").into()
        } else {
            self.send_hint(cx)
        };
        let submit = if self.working() || cancelling {
            gds_uim::el! {
                Button("stop-turn") {
                    ghost;
                    small;
                    icon: $IconName::Square;
                    text_color: $cx.theme().danger;
                    @$self.composer_error(cx).is_some() => {
                        text_color: $cx.theme().danger;
                    };
                    disabled: $(
                    cancelling || self.connection_status() != status::ConnectionStatus::Connected
                    );
                    on_click: $cx.listener(Self::on_composer_stop);
                }
            }
        } else {
            gds_uim::el! {
                Button("send-message") {
                    primary;
                    @$(!can_send) => { ghost; };
                    small;
                    icon: $IconName::Send;
                    @$self.composer_error(cx).is_some() => { text_color: $cx.theme().danger; };
                    disabled: $(!can_send);
                    on_click: $cx.listener(Self::on_composer_send);
                }
            }
        };
        let panel = gds_uim::el! {
            #div {
                id: "composer-panel";
                test_support;
                flex;
                flex_col;
                @fills_thread => { flex_1; min_h_0; };
            }
        }
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, cx| {
            style.bg(cx.theme().primary.opacity(0.12))
        })
        .on_drop(cx.listener(Self::on_composer_drop));
        gds_uim::el! {
            #div {
                flex_shrink_0;
                w_full;
                @fills_thread => { flex_1; min_h_0; flex; flex_col; };
                @$self.window.compact => {
                    px: $px(COMPOSER_SIDE_PADDING);
                    pb: $px(COMPOSER_BOTTOM_PADDING);
                };
                @$(!self.window.compact) => { pb: $px(COMPOSER_SIDE_PADDING); };
            } [
                $panel {
                    @$(!self.window.compact) => { border_t_1; border_color: $cx.theme().border; };
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
        let editor = gds_uim::el! {
            Editor($&self.composer.input) {
                w_auto; flex_1; min_w_0;
                ml: $inset;
                font_family: $cx.theme().font_family.clone();
                text_size: $theme::content_font_size(cx);
                line_height: $gpui_kit::relative(theme::content_line_height(cx));
                appearance: false;
                bordered: false;
                readonly: $(composition_locked || self.composer.paste_pending);
                context_menu: $move |menu, _, cx| super::text_menu(menu, editable, copyable, cx);
                aria_label: $crate::i18n::tr!("composer-message");
                @fills_thread => { h_full; };
                @$(!fills_thread) => { h: $px(self.composer_height(cx)); };
            }
        };
        gds_uim::el! {
            #div {
                id: "composer-editor";
                relative;
                key_context: "BasComposer";
                test_support;
                capture_action: $cx.listener(Self::on_composer_expand_action);
                capture_action: $cx.listener(Self::on_composer_send_action);
                capture_action: $cx.listener(Self::capture_composer_enter);
                capture_action: $cx.listener(Self::on_composer_paste_action);
                capture_action: $cx.listener(Self::on_composer_backspace_action);
                capture_action: $cx.listener(Self::on_composer_delete_action);
                capture_action: $cx.listener(Self::on_composer_previous_word_action);
                capture_action: $cx.listener(Self::on_composer_next_word_action);
                capture_action: $cx.listener(Self::on_composer_left_action);
                capture_action: $cx.listener(Self::on_composer_right_action);
                capture_action: $cx.listener(Self::on_composer_up_action);
                capture_action: $cx.listener(Self::on_composer_down_action);
                capture_action: $cx.listener(Self::on_composer_tab_action);
                capture_action: $cx.listener(Self::on_composer_escape_action);
                flex;
                items_start;
                pr: $px(COMPOSER_CONTROL_INSET + layout::ICON_BUTTON_SIZE + COMPOSER_CONTROL_GAP);
                @fills_thread => { flex_1; min_h_0; };
            } [
                #div {
                    relative; flex; flex_1; min_w_0; overflow_x_hidden;
                    @fills_thread => { h_full; };
                } [$editor, $self.render_attachment_chips(cx)],
                #div {
                    absolute;
                    top: $expand_top.max(px(0.));
                    right: $px(COMPOSER_CONTROL_INSET);
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
        gds_uim::el! {
            #div {
                id: "composer-toolbar";
                test_support; w_full; flex; items_center; gap_1;
                pl: $px(COMPOSER_CONTROL_INSET);
                pr: $px(COMPOSER_CONTROL_INSET);
                h: $px(COMPOSER_TOOLBAR_HEIGHT);
                flex_shrink_0;
            } [
                ..$self.render_usage(cx),
                $self.attachment_button(cx) {
                    w: $px(layout::ICON_BUTTON_SIZE); h: $px(layout::ICON_BUTTON_SIZE); p_0;
                },
                #div { flex_1; min_w_0; } [$self.render_config_options(cx)],
                ..$self.render_queue(cx),
                ..$((self.working() && !self.composer.input.read(cx).value().trim().is_empty())
                    .then(|| self.immediate_send_button(cx))),
                ..$((!self.window.compact && self.selected.authentication.required.is_some())
                    .then(|| self.authentication_button(cx))),
                ..$((self.working() && !self.composer.input.read(cx).value().trim().is_empty())
                    .then(|| crate::ui::tooltip::bounded(
                        "queue-message-hint",
                        gds_uim::el! {
                            Button("queue-message") {
                                ghost; xsmall; icon: $IconName::Send;
                                disabled: $(!self.can_send(cx));
                                on_click: $cx.listener(Self::on_composer_send);
                            }
                        },
                        self.send_hint(cx),
                    ))),
                $crate::ui::tooltip::bounded(
                    "submit-hint",
                    gds_uim::el! {
                        $submit {
                            xsmall;
                            w: $px(layout::ICON_BUTTON_SIZE);
                            h: $px(layout::ICON_BUTTON_SIZE);
                            p_0;
                            flex_shrink_0;
                        }
                    },
                    submit_hint,
                ),
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
