//! Opens settings windows and records the workspace targeted by agent changes.

use super::*;

impl Workspace {
    pub(super) fn open_agent_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        if let Err(error) = crate::settings::window::open_catalog(cx) {
            self.selected.error = Some(crate::i18n::tr!(
                "catalog-open-error",
                error = (error).to_string()
            ));
        }
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        if let Err(error) = crate::settings::window::open(cx) {
            self.selected.error = Some(crate::i18n::tr!(
                "settings-open-error",
                error = (error).to_string()
            ));
        }
        cx.notify();
    }

    pub(super) fn record_settings_target(&self, cx: &mut Context<Self>) {
        crate::connections::manage(self.backend.server.clone(), cx);
        cx.update_global::<crate::agents::Changes, _>(|changes, _| {
            changes.target = Some(self.window.handle.window_id());
            changes.select = None;
            changes.ready = None;
            changes.server = None;
        });
    }
}
