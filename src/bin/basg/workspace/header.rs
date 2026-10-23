use crate::ui::window_controls::WindowControls;
use gpui_kit::ClickEvent;

use super::*;

impl Workspace {
    fn title_slot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        el! {
            .div {
                # "thread-title-slot";
                test_support;
                flex_1;
                min_w_0;
                h_8;
                flex;
                items_center;
                gap_1;
            } [
                @if (self.renaming_header())
                    $self.render_title_editor(cx),
                @else
                    .div {
                        # "rename-session";
                        test_support;
                        flex_1;
                        min_w_0;
                        truncate;
                        text_size: ui::SMALL;
                        cursor_pointer;
                        hoverable_tooltip: $shortcuts::tooltip(tr!("session-rename"), shortcuts::RenameThread);
                        on_click: $cx.listener(Self::on_header_rename);
                    } [
                        $self.current().map(title).unwrap_or_else(|| self.draft_title()),
                    ],
                @end,
            ]
        }
    }

    pub(super) fn render_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        el! {
            .div {
                # "thread-header";
                test_support;
                flex;
                flex_col;
                flex_shrink_0;
            } [
                $self.header_row(window, cx),
                @if (let Some(session) = self.current().filter(|session| session.archived))
                    .div {
                        px_5;
                        pb_3;
                        flex;
                        items_center;
                        gap_3;
                    } [
                        .div { flex_1; } [tr!("session-archived-help")],
                        $self.archive_button(session, "restore-archived", cx),
                    ],
                @end,
            ]
        }
    }

    fn header_row(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let show_application = !self.window.sidebar_visible;
        let controls = WindowControls::new()
            .on_maximize(cx.listener(Self::on_window_zoom))
            .on_close_window(cx.listener(Self::on_window_close));
        el! {
            .div {
                # "conversation-header-row";
                test_support;
                flex;
                flex_shrink_0;
                h: layout::HEADER_HEIGHT.px;
                items_center;
                gap_1;
                px_3;
                @if (cfg!(target_os = "macos") && !self.window.sidebar_visible)
                    pl: layout::MACOS_WINDOW_CONTROLS_WIDTH.px;
                @end;
            } [
                @if (show_application)
                    $self.application_button(cx),
                @end,
                ("sidebar-toggle", IconName::PanelLeft).icon_button {
                    tooltip: tr!("sidebar-toggle");
                    on_click: $cx.listener(Self::on_sidebar_toggle);
                },
                $self.workspace_button(cx),
                ("/", cx).caption,
                $self.title_slot(cx),
                $self.mode_button(cx),
                controls,
            ]
        }
    }

    fn on_header_rename(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_rename(window, cx);
    }

    fn on_sidebar_toggle(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar(window, cx);
    }

    fn on_window_zoom(&mut self, _: &ClickEvent, window: &mut Window, _: &mut Context<Self>) {
        if !window.is_maximized() {
            self.window.size = window.bounds().size;
        }
        window.zoom_window();
    }

    fn on_window_close(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.close_window(window, cx);
    }
}
