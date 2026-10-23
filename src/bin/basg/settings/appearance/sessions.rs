//! Session preferences share the local editor, not the appearance page's navigation.

use super::*;
use crate::settings::form::Row;

const DEFAULT_AGENT: Section =
    Section::new("settings-default-agent", "settings-default-agent-help");
const AUTO_COLLAPSE: Section = Section::new(
    "settings-auto-collapse-activity",
    "settings-auto-collapse-activity-help",
);
const CONFIRM_DELETE: Section =
    Section::new("settings-confirm-delete", "settings-confirm-delete-help");

impl AppearancePage {
    pub(in crate::settings) fn search_sessions(query: &Query) -> Option<Match> {
        [
            Category::Sessions.section(),
            DEFAULT_AGENT,
            AUTO_COLLAPSE,
            CONFIRM_DELETE,
            Field::ComposerEnter.section(),
        ]
        .into_iter()
        .filter_map(|section| section.score_under(query, &[Category::Sessions.section()]))
        .max()
    }

    pub(in crate::settings) fn matches_session_query(query: &str) -> bool {
        let query = Query::new(query);
        query.is_empty() || Self::search_sessions(&query).is_some()
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
            label: DEFAULT_AGENT.title(),
            help: DEFAULT_AGENT.help(),
            control: el! { .div { # "settings-default-agent"; test_support; flex_1; min_w_0; } [
                ($&self.form.default_agent).Select::new { small; w_full; cursor_pointer; disabled: $self.document.is_none(); },
            ] }.into_any_element(),
            reset: None,
            last: false,
        };
        let auto_collapse_activity = Row {
            id: "settings-auto-collapse-activity".into(),
            label: AUTO_COLLAPSE.title(),
            help: AUTO_COLLAPSE.help(),
            control: el! {
                ("settings-auto-collapse-activity").ui::checkbox {
                    accessibility_label: $AUTO_COLLAPSE.title();
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
            label: CONFIRM_DELETE.title(),
            help: CONFIRM_DELETE.help(),
            control: el! {
                ("settings-confirm-delete").ui::checkbox {
                    accessibility_label: $CONFIRM_DELETE.title();
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
        el! { .div {
            # "session-settings"; test_support;
            size_full; overflow_y_scroll; p_5; flex; flex_col; gap_2;
        } [
            .div { flex; flex_col; gap_2; } [
                .div { text_size: ui::HEADING; } [$Category::Sessions.title()],
                .div { text_size: ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                    $Category::Sessions.section().help(),
                ],
            ],
            @if (Self::matches_session_query(&self.query))
                .div { border_t_1; border_color: $cx.theme().border; flex; flex_col; } [
                    default_agent,
                    $self.row(Field::ComposerEnter, false, cx),
                    auto_collapse_activity,
                    confirm_delete,
                ],
            @else
                .div { # "settings-no-results"; test_support; } [tr!("settings-no-results")],
            @end,
        ] }
        .into_any_element()
    }
}
