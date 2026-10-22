//! Session preferences share the local editor, not the appearance page's navigation.

use super::*;
use crate::settings::form::Row;

impl AppearancePage {
    pub(in crate::settings) fn matches_session_query(query: &str) -> bool {
        let query = query.trim().to_lowercase();
        [
            crate::i18n::tr!("settings-sessions"),
            crate::i18n::tr!("settings-sessions-help"),
            crate::i18n::tr!("settings-confirm-delete"),
            crate::i18n::tr!("settings-default-agent"),
            crate::i18n::tr!("settings-auto-collapse-activity"),
            crate::i18n::tr!("settings-auto-collapse-activity-help"),
            Field::ComposerEnter.label(),
            Field::ComposerEnter.help(),
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
        let agents = crate::settings::agents::shared(window, cx)
            .read(cx)
            .display_catalog();
        let selected = self
            .form
            .default_agent
            .read(cx)
            .selected_value()
            .cloned()
            .flatten();
        let mut choices: Vec<_> = std::iter::once(None)
            .chain(agents.iter().map(|spec| Some(spec.id.to_string())))
            .chain(selected.clone().map(Some))
            .collect();
        choices.sort();
        choices.dedup();
        if choices != self.form.default_agent_choices {
            self.form.default_agent_choices = choices.clone();
            self.form.default_agent.update(cx, |select, cx| {
                select.set_items(choices.into_iter().map(form::Choice).collect(), window, cx);
                select.set_selected_value(&selected, window, cx);
            });
        }
        let default_agent = Row {
            id: "settings-default-agent".into(),
            label: crate::i18n::tr!("settings-default-agent"),
            help: crate::i18n::tr!("settings-default-agent-help"),
            control: gds_uim::el! { #div { id: "settings-default-agent"; test_support; flex_1; min_w_0; } [
                Select($&self.form.default_agent) { small; w_full; cursor_pointer; disabled: $self.document.is_none(); },
            ] }.into_any_element(),
            reset: None,
            last: false,
        };
        let auto_collapse_activity = Row {
            id: "settings-auto-collapse-activity".into(),
            label: crate::i18n::tr!("settings-auto-collapse-activity"),
            help: crate::i18n::tr!("settings-auto-collapse-activity-help"),
            control: gds_uim::el! {
                #crate::ui::checkbox("settings-auto-collapse-activity") {
                    accessibility_label: $crate::i18n::tr!("settings-auto-collapse-activity");
                    checked: $self.form.auto_collapse_activity.unwrap_or(true);
                    disabled: $self.document.is_none();
                    on_click: $cx.listener(|this, checked: &bool, window, cx| {
                        this.form.auto_collapse_activity = Some(*checked);
                        this.flush_pending(window, cx);
                    });
                }
            }
            .into_any_element(),
            reset: None,
            last: false,
        };
        let confirm_delete = Row {
            id: "settings-confirm-delete".into(),
            label: crate::i18n::tr!("settings-confirm-delete"),
            help: crate::i18n::tr!("settings-confirm-delete-help"),
            control: gds_uim::el! {
                #crate::ui::checkbox("settings-confirm-delete") {
                    accessibility_label: $crate::i18n::tr!("settings-confirm-delete");
                    checked: $!self.form.confirm_delete.unwrap_or(true);
                    disabled: $self.document.is_none();
                    on_click: $cx.listener(|this, checked: &bool, window, cx| {
                        this.form.confirm_delete = Some(!*checked);
                        this.flush_pending(window, cx);
                    });
                }
            }
            .into_any_element(),
            reset: None,
            last: true,
        };
        gds_uim::el! { #div {
            id: "session-settings"; test_support;
            size_full; overflow_y_scroll; p_5; flex; flex_col; gap_2;
        } [
            #div { flex; flex_col; gap_2; } [
                #div { text_size: crate::ui::HEADING; } [$crate::i18n::tr!("settings-sessions")],
                #div { text_size: crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                    $crate::i18n::tr!("settings-sessions-help"),
                ],
            ],
            ..$bool::then(Self::matches_session_query(&self.query), || gds_uim::el! {
                #div { border_t_1; border_color: $cx.theme().border; flex; flex_col; } [
                    $default_agent,
                    $self.row(Field::ComposerEnter, false, cx),
                    $auto_collapse_activity,
                    $confirm_delete,
                ]
            }),
            ..$bool::then(!Self::matches_session_query(&self.query), || gds_uim::el! {
                #div { id: "settings-no-results"; test_support; } [$crate::i18n::tr!("settings-no-results")]
            }),
        ] }.into_any_element()
    }
}
