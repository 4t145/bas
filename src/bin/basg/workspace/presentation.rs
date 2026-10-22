use super::*;

impl Workspace {
    pub(crate) fn connection_options(&self) -> &Target {
        &self.backend.server
    }

    pub(crate) fn is_compact(&self) -> bool {
        self.window.compact
    }

    pub(crate) fn show_in_window(
        &mut self,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window.compact != compact {
            self.window.compact = compact;
            self.window.sidebar_visible = !compact;
            self.sync_composer_mode(cx);
        }
        self.set_presented(true, window, cx);
    }

    pub(crate) fn set_presented(
        &mut self,
        presented: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window.presented = presented;
        if presented {
            if !self.window.compact {
                self.window.size = window.bounds().size;
            }
            self.window.popup_size = None;
            self.resize_window(window, cx);
            self.composer
                .input
                .update(cx, |input, cx| input.focus(window, cx));
            self.resume_workspace_picker(window, cx);
        } else {
            self.prepare_to_close(window, cx);
        }
        self.sync_foreground(window);
        cx.notify();
    }
}
