//! Session preferences share the local editor, not the appearance page's navigation.

use super::*;

impl AppearancePage {
    pub(in crate::settings) fn matches_session_query(query: &str) -> bool {
        let query = query.trim().to_lowercase();
        [
            crate::i18n::tr!("settings-sessions"),
            crate::i18n::tr!("settings-sessions-help"),
            crate::i18n::tr!("settings-confirm-delete"),
        ]
        .iter()
        .any(|text| text.to_lowercase().contains(&query))
    }

    pub(in crate::settings) fn render_sessions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if self.reload {
            self.refresh(window, cx);
        }
        gds_uim::el! { #div {
            id: "session-settings"; test_support;
            size_full; overflow_y_scroll; p_5; flex; flex_col; gap_5;
        } [
            #div { flex; flex_col; gap_2; } [
                #div { text_size: crate::ui::HEADING; } [$crate::i18n::tr!("settings-sessions")],
                #div { text_size: crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                    $crate::i18n::tr!("settings-sessions-help"),
                ],
            ],
            ..$bool::then(Self::matches_session_query(&self.query), || gds_uim::el! {
                #div { border_t_1; border_color: $cx.theme().border; py_4; flex; flex_col; gap_2; } [
                    #crate::ui::checkbox("settings-confirm-delete") {
                        label: $crate::i18n::tr!("settings-confirm-delete");
                        checked: $!self.form.confirm_delete.unwrap_or(true);
                        disabled: $self.document.is_none();
                        on_click: $cx.listener(|this, checked: &bool, window, cx| {
                            this.form.confirm_delete = Some(!*checked);
                            this.flush_pending(window, cx);
                        });
                    },
                    #div { text_size: crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                        $crate::i18n::tr!("settings-confirm-delete-help"),
                    ],
                ]
            }),
            ..$bool::then(!Self::matches_session_query(&self.query), || gds_uim::el! {
                #div { id: "settings-no-results"; test_support; } [$crate::i18n::tr!("settings-no-results")]
            }),
            ..$bool::then(!self.message.is_empty(), || gds_uim::el! {
                #div { text_size: crate::ui::SMALL; } [$self.message.clone()]
            }),
        ] }.into_any_element()
    }
}
