use super::*;

const POPUP_WIDTH: f32 = 640.;
pub(super) const INPUT_HEIGHT: f32 = 98.;
const COMPACT_HEADER_HEIGHT: f32 = 28.;
const PANEL_BORDER_WIDTH: f32 = 1.;
const COMMAND_ROW_HEIGHT: f32 = 44.;
const COMMAND_HINT_HEIGHT: f32 = 28.;
const OUTPUT_HEIGHT: f32 = 440.;
pub(super) const HEADER_HEIGHT: f32 = 44.;
pub(super) const ICON_BUTTON_SIZE: f32 = 24.;
pub(super) const MACOS_WINDOW_CONTROLS_WIDTH: f32 = 80.;

pub(super) fn icon_button(id: impl Into<gpui_kit::ElementId>, icon: IconName) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .w(px(ICON_BUTTON_SIZE))
        .h(px(ICON_BUTTON_SIZE))
        .p_0()
        .flex_shrink_0()
}

pub(crate) fn initial_size(compact: bool) -> Size<Pixels> {
    if compact {
        size(px(POPUP_WIDTH), px(INPUT_HEIGHT))
    } else {
        size(px(1120.), px(800.))
    }
}

impl Workspace {
    pub(super) fn prepare_to_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_title(cx);
        self.remember_thread();
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
    }

    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Programmatic removal does not invoke the native should-close callback.
        self.prepare_to_close(window, cx);
        window.remove_window();
    }

    pub(super) fn resize_window(&mut self, window: &mut Window, cx: &App) {
        let command_count = self
            .command_candidates(cx)
            .len()
            .min(commands::VISIBLE_COMMANDS);
        let command_height = command_count as f32 * COMMAND_ROW_HEIGHT
            + if command_count > 0 || self.command_hint(cx).is_some() {
                COMMAND_HINT_HEIGHT
            } else {
                0.
            };
        let menu_height = self
            .menu
            .as_ref()
            .filter(|menu| menu.window.is_none())
            .map_or(0., |menu| f32::from(menu.size.height));
        let menu_height = menu_height
            + self
                .reply_preview
                .as_ref()
                .filter(|preview| preview.window.is_none())
                .map_or(0., |preview| f32::from(preview.size.height));
        let size = if self.compact {
            size(
                px(POPUP_WIDTH),
                px(if self.output_open {
                    OUTPUT_HEIGHT.max(
                        COMPACT_HEADER_HEIGHT
                            + self.composer_height(cx)
                            + chat::COMPOSER_TOOLBAR_HEIGHT
                            + chat::COMPOSER_BOTTOM_PADDING
                            + PANEL_BORDER_WIDTH * 2.
                            + 100.,
                    ) + command_height
                        + menu_height
                } else {
                    COMPACT_HEADER_HEIGHT
                        + self.composer_height(cx)
                        + chat::COMPOSER_TOOLBAR_HEIGHT
                        + chat::COMPOSER_BOTTOM_PADDING
                        + PANEL_BORDER_WIDTH * 2.
                        + command_height
                        + menu_height
                }),
            )
        } else {
            self.window_size
        };
        if self.compact && self.popup_size == Some(size) {
            return;
        }
        self.popup_size = self.compact.then_some(size);
        if window.bounds().size != size {
            window.resize(size);
        }
        window.set_window_title(if self.compact {
            "BASR — Quick input"
        } else {
            "BASR"
        });
        if let Some(placement) = &mut self.placement {
            placement.update(self.compact, size);
        }
    }

    pub(super) fn toggle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_title(cx);
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.usage_details_open = None;
        if !self.compact {
            if window.is_maximized() {
                window.zoom_window();
            } else {
                self.window_size = window.bounds().size;
            }
        }
        self.compact = !self.compact;
        if !self.compact {
            self.sidebar_visible = true;
            if !self.thread_has_user_prompt() {
                self.composer_expanded = true;
                self.sync_composer_mode(cx);
            }
        }
        self.resize_window(window, cx);
        self.list.remeasure_items(
            0..self
                .conversation
                .as_ref()
                .map_or(0, |state| state.rows.len()),
        );
        self.focus_input(window, cx);
        cx.notify();
    }

    pub(super) fn toggle_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.compact {
            self.close_reply_preview(window, cx);
            self.output_open = !self.output_open;
            if self.output_open {
                self.list.set_follow_mode(FollowMode::Tail);
            }
            self.resize_window(window, cx);
            cx.notify();
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cancel_queue_edit(window, cx) {
            return;
        }
        if self.reply_preview.is_some() {
            self.close_reply_preview(window, cx);
        } else if self.menu.is_some() {
            self.close_menu(window, cx);
        } else if !self.command_candidates(cx).is_empty() {
            self.commands_dismissed = true;
            if self.compact {
                self.resize_window(window, cx);
            }
            cx.notify();
        } else if self.title_edit.is_some() {
            self.cancel_rename(window, cx);
        } else if !self.compact {
            self.toggle_mode(window, cx);
        } else if self.output_open {
            self.toggle_output(window, cx);
        } else {
            self.close_window(window, cx);
        }
    }

    pub(super) fn mode_button(&self, cx: &mut Context<Self>) -> Button {
        icon_button(
            "toggle-mode",
            if self.compact {
                IconName::Maximize2
            } else {
                IconName::Minimize2
            },
        )
        .tooltip(if self.compact {
            "展开窗口 · Ctrl Shift M"
        } else {
            "最小模式 · Ctrl Shift M"
        })
        .on_click(cx.listener(|this, _, window, cx| this.toggle_mode(window, cx)))
    }

    pub(super) fn render_layout(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(left) = crate::input_geometry::content_left(
            &self.composer,
            gpui_kit::component::Size::Medium.input_px(),
            window,
            cx,
        ) {
            self.composer_content_left.set(left);
        }
        self.composer_available_height
            .set((f32::from(window.viewport_size().height) - 180.).max(80.));
        div()
            .id("basr-workspace")
            .key_context("BasrWorkspace")
            .size_full()
            .flex()
            .refine_style(&theme::font_style(cx))
            .text_color(cx.theme().foreground)
            .capture_any_mouse_down(cx.listener(
                |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.close_menu_outside(event.position, window, cx);
                },
            ))
            .on_action(|_: &gpui_kit::component::input::Enter, _, _| {})
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .on_action(
                cx.listener(|this, _: &NextThread, window, cx| {
                    this.cycle_thread(false, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &PreviousThread, window, cx| {
                this.cycle_thread(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FocusComposer, window, cx| this.focus_input(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.compact {
                    this.toggle_mode(window, cx);
                }
                this.sidebar_visible = true;
                this.search.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleMode, window, cx| this.toggle_mode(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleOutput, window, cx| this.toggle_output(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ScrollConversationToEnd, _, cx| {
                this.scroll_conversation_to_end(cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .child(if self.compact {
                self.render_popup(cx)
            } else {
                self.render_window(window, cx)
            })
            .children(self.inline_context_menu())
            .into_any_element()
    }

    fn render_popup(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("minimal-mode")
            .size_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .overflow_hidden()
            .border_1()
            .border_color(cx.theme().border)
            .bg(theme::content_background(cx))
            .when_some(
                self.reply_preview
                    .as_ref()
                    .filter(|preview| preview.window.is_none()),
                |panel, preview| {
                    panel.child(
                        div()
                            .flex_shrink_0()
                            .h(preview.size.height)
                            .child(preview.content.clone()),
                    )
                },
            )
            .when_some(
                self.menu.as_ref().filter(|menu| menu.window.is_none()),
                |panel, menu| {
                    panel.child(
                        div()
                            .flex_shrink_0()
                            .h(menu.size.height)
                            .child(menu.content.clone()),
                    )
                },
            )
            .when(self.output_open, |panel| {
                panel.child(
                    div()
                        .id("agent-output")
                        .test_support()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(self.render_timeline(cx)),
                )
            })
            .child(
                div()
                    .id("compact-header")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_1()
                    .mx(px(chat::COMPOSER_SIDE_PADDING))
                    .pl(px(chat::COMPOSER_CONTROL_INSET))
                    .pr(px(chat::COMPOSER_CONTROL_INSET))
                    .when(cfg!(target_os = "macos"), |header| {
                        header.pl(px(MACOS_WINDOW_CONTROLS_WIDTH))
                    })
                    .h(px(COMPACT_HEADER_HEIGHT))
                    .flex_shrink_0()
                    .child(
                        icon_button(
                            "toggle-output",
                            if self.output_open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                        )
                        .tooltip("折叠 / 展开输出 · Ctrl O")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_output(window, cx)),
                        ),
                    )
                    .child(self.agent_button(cx))
                    .child(
                        div()
                            .id("compact-status")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .child(self.connection_indicator("compact-connection", cx))
                            .when(
                                self.current().is_some_and(|session| session.archived),
                                |status| status.child(caption("已归档", cx)),
                            )
                            .children(
                                self.activity_label()
                                    .map(|label| caption(label, cx).flex_1().min_w_0().truncate()),
                            )
                            .when(self.activity_label().is_none(), |status| {
                                status.children(self.render_reply_summary(cx))
                            })
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
                                window.start_window_move()
                            }),
                    )
                    .when(self.active.is_some() && self.output_open, |header| {
                        header.child(self.render_conversation_navigation(cx))
                    })
                    .when_some(
                        self.current().filter(|session| session.archived),
                        |row, session| {
                            row.child(self.archive_button(session, "compact-restore", cx))
                        },
                    )
                    .when(self.authentication_required.is_some(), |row| {
                        row.child(self.authentication_button(cx))
                    })
                    .child(self.application_button(cx))
                    .child(self.workspace_button(cx))
                    .child(
                        icon_button("new-session", IconName::Plus)
                            .tooltip("新 thread · Ctrl N")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    )
                    .child(self.mode_button(cx)),
            )
            .child(self.render_composer(cx))
            .into_any_element()
    }

    fn render_window(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("window-mode")
            .size_full()
            .flex()
            .when(self.sidebar_visible, |row| {
                row.child(self.render_sidebar(window, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(theme::content_background(cx))
                    .child(self.render_header(window, cx))
                    .child(self.render_chat(cx)),
            )
            .into_any_element()
    }
}
