use super::*;
use crate::ui::{Button, icon_action, session_status, time};
use bas::model::Timestamp;
use gpui_kit::{
    AnyElement, Render, TestSupportExt,
    assets::IconName,
    base::{Disableable, Selectable},
    component::{
        ActiveTheme, Icon, Sizable,
        button::*,
        input::Input,
        menu::{DropdownMenu, PopupMenuItem},
        spinner::Spinner,
    },
    div,
    prelude::*,
    px,
};
use std::cmp::Reverse;

const AGENT_WIDTH: f32 = 184.;

impl Browser {
    fn progress(&self, id: &'static str, message: &str, cx: &App) -> AnyElement {
        gds_uim::el! { #div {
            id: id; test_support; flex; items_center; gap_2; py_3;
            role: gpui_kit::Role::Status; aria_label: $message.to_owned();
            text_color: $cx.theme().muted_foreground;
        } [
            #div {
                id: $format!("{id}-spinner"); test_support; flex; flex_shrink_0; size: crate::ui::ICON.px;
            } [
                Spinner { icon: IconName::LoaderCircle; with_size: crate::ui::ICON.px; color: $cx.theme().primary; },
            ],
            $message.to_owned(),
        ] }.into_any_element()
    }

    fn agent_icon(&self, id: &str, cx: &App) -> AnyElement {
        let artwork = crate::agents::metadata::entity(cx)
            .and_then(|metadata| metadata.read(cx).icon(&self.server, id))
            .unwrap_or_default();
        gds_uim::el! {
            crate::agents::icon::AgentIcon($format!("history-agent-icon-{id}"), artwork) {
                size: crate::ui::ICON.px;
            }
        }
        .into_any_element()
    }

    fn agent_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let agents = self.agents.iter().enumerate().map(|(index, agent)| {
            let id = agent.id.clone();
            let title = id.clone();
            gds_uim::el! { Button($format!("agent-history-filter-{index}")) {
                ghost; small; w_full; selected: $self.agent.as_ref() == Some(&id);
                accessibility_label: $id.clone(); tooltip: $id.clone();
                on_click: $cx.listener(move |this, _, _, cx| this.select_agent(Some(id.clone()), cx));
            } [
                $self.agent_icon(&title, cx),
                #div { flex_1; min_w_0; text_left; truncate; text_size: $crate::ui::SMALL; } [$title],
            ] }
        }).collect::<Vec<_>>();
        gds_uim::el! { #div {
            id: "agent-history-agents"; test_support; w: AGENT_WIDTH.px; h_full;
            flex; flex_col; flex_shrink_0; gap_1; p_3;
            bg: $cx.theme().sidebar; border_r_1; border_color: $cx.theme().border;
        } [
            #div { text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground; pb_2; } [
                $crate::i18n::tr!("agent-history-agent-filter"),
            ],
            Button("agent-history-all-agents") {
                ghost; small; w_full;
                accessibility_label: $crate::i18n::tr!("agent-history-all-agents"); selected: $self.agent.is_none();
                on_click: $cx.listener(|this, _, _, cx| this.select_agent(None, cx));
            } [
                Icon(IconName::Bot) { size: crate::ui::ICON.px; },
                #div { flex_1; min_w_0; text_left; truncate; text_size: $crate::ui::SMALL; } [
                    $crate::i18n::tr!("agent-history-all-agents"),
                ],
            ],
            #div { id: "agent-history-agent-list"; flex_1; min_h_0; overflow_y_scroll; } [..agents],
        ] }.into_any_element()
    }

    fn workspace_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices: Vec<_> = self.workspaces.iter().cloned().collect();
        let selected = self.cwd.clone();
        let hint = selected
            .as_ref()
            .map(|cwd| cwd.display().to_string())
            .unwrap_or_else(|| crate::i18n::tr!("agent-history-all-workspaces").into());
        let owner = cx.entity().downgrade();
        let label = hint.clone();
        let button = gds_uim::el! { Button("agent-history-workspace-filter") {
            small; ghost; w_full; min_w_0;
            @$(selected.is_some()) => { icon: IconName::Folder; };
            tooltip: hint; accessibility_label: $crate::i18n::tr!("agent-history-workspace-filter");
        } [
            #div { flex_1; min_w_0; truncate; } [$label],
            Icon(IconName::ChevronDown) { xsmall; },
        ] };
        gds_uim::el! { $button {
            dropdown_menu: $move |menu, _, _| {
                let all_owner = owner.clone();
                let mut menu = crate::ui::menu(menu).item(
                    PopupMenuItem::new(crate::i18n::tr!("agent-history-all-workspaces"))
                        .checked(selected.is_none())
                        .on_click(move |_, _, cx| { let _ = all_owner.update(cx, |this, cx| this.select_workspace(None, cx)); }),
                );
                for cwd in &choices {
                    let path = cwd.clone();
                    let view = owner.clone();
                    menu = menu.item(PopupMenuItem::new(cwd.display().to_string())
                        .icon(IconName::Folder).checked(selected.as_ref() == Some(cwd))
                        .on_click(move |_, _, cx| { let _ = view.update(cx, |this, cx| this.select_workspace(Some(path.clone()), cx)); }));
                }
                menu
            };
        } }.into_any_element()
    }

    pub(super) fn visible_rows<'a>(
        &self,
        agent: &str,
        page: &'a connection::AgentHistory,
        query: &str,
    ) -> Vec<&'a acp::SessionInfo> {
        let mut rows: Vec<_> = page
            .sessions
            .sessions
            .iter()
            .filter(|info| {
                self.cwd.as_ref().is_none_or(|cwd| &info.cwd == cwd)
                    && format!(
                        "{} {} {} {}",
                        agent,
                        info.title.as_deref().unwrap_or(""),
                        info.cwd.display(),
                        info.session_id
                    )
                    .to_lowercase()
                    .contains(query)
            })
            .collect();
        rows.sort_by_key(|info| {
            (
                Reverse(info.updated_at.as_deref().and_then(time::parse_rfc3339)),
                info.session_id.to_string(),
            )
        });
        rows
    }

    fn groups(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_lowercase();
        self.agents.iter().enumerate()
            .filter(|(_, agent)| self.agent.as_ref().is_none_or(|selected| selected == &agent.id))
            .filter_map(|(agent_index, agent)| {
                let page = self.pages.get(&agent.id);
                let rows = page.map(|page| self.visible_rows(&agent.id, page, &query)).unwrap_or_default();
                let error = self.errors.get(&agent.id);
                if rows.is_empty() && error.is_none() { return None; }
                let count = rows.len();
                let entries = rows.into_iter().enumerate().map(|(index, info)| {
                    self.row(format!("{agent_index}-{index}"), &agent.id, info, page.expect("rows have page"), cx)
                }).collect::<Vec<_>>();
                Some(gds_uim::el! { #div {
                    id: $format!("agent-history-group-{agent_index}"); test_support; flex; flex_col; gap_2; mb_4;
                } [
                    #div { flex; items_center; gap_2; py_2; } [
                        $self.agent_icon(&agent.id, cx),
                        #div { id: $format!("agent-history-group-name-{agent_index}"); test_support; min_w_0; truncate; } [$agent.id.clone()],
                        #div { id: $format!("agent-history-group-count-{agent_index}"); test_support; text_color: $cx.theme().muted_foreground; } [$count.to_string()],
                    ],
                    ..$error.map(|error| crate::ui::feedback(
                        format!("agent-history-error-{agent_index}"), error.clone(), cx.theme().danger,
                    )),
                    ..entries,
                ] }.into_any_element())
            }).collect()
    }

    fn row_actions(
        &self,
        key: String,
        agent: &str,
        info: &acp::SessionInfo,
        page: &connection::AgentHistory,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = Target {
            generation: self.generation,
            agent: agent.into(),
            info: info.clone(),
        };
        let imported = self.imported(agent, info);
        let active = self.active(&target);
        let capabilities = &page.initialization.agent_capabilities;
        let resumable =
            capabilities.load_session || capabilities.session_capabilities.resume.is_some();
        let importable =
            imported.is_some() || (resumable && info.additional_directories.is_empty());
        let busy = self.operation.is_some() || self.connection_error.is_some();
        let primary = self.primary_action(agent, info);
        let open_label = if imported.is_some() {
            crate::i18n::tr!("agent-history-open")
        } else if capabilities.load_session {
            crate::i18n::tr!("agent-history-import")
        } else {
            crate::i18n::tr!("agent-history-resume")
        };
        let primary_target = target.clone();
        let close_target = target.clone();
        let (suffix, icon, label, hint, allowed) = match primary {
            PrimaryAction::Open => (
                "import",
                IconName::MessageSquare,
                open_label,
                open_label,
                importable,
            ),
            PrimaryAction::Restart => (
                "restart",
                IconName::Play,
                crate::i18n::tr!("agent-session-restart"),
                if resumable {
                    crate::i18n::tr!("agent-session-restart-help")
                } else {
                    crate::i18n::tr!("agent-history-cannot-import")
                },
                resumable,
            ),
            PrimaryAction::Restore => (
                "restore",
                IconName::ArchiveRestore,
                crate::i18n::tr!("agent-history-restore"),
                crate::i18n::tr!("agent-history-restore-help"),
                true,
            ),
        };
        let loading = match &self.operation {
            Some(Operation::Import { target, .. }) => {
                target.agent == agent && target.info.session_id == info.session_id
            }
            Some(Operation::Restart { session, .. } | Operation::Restore { session, .. }) => {
                imported.is_some_and(|snapshot| snapshot.meta.id == *session)
            }
            _ => false,
        };
        let close = active.then(|| gds_uim::el! {
            #icon_action($format!("agent-history-close-{key}"), IconName::Square, $crate::i18n::tr!("agent-session-close")) {
                disabled: busy;
                on_click: $cx.listener(move |this, _, _, cx| this.close(close_target.clone(), cx));
            }
        });
        let delete = capabilities.session_capabilities.delete.is_some().then(|| gds_uim::el! {
            #icon_action($format!("agent-history-delete-{key}"), IconName::Trash, $crate::i18n::tr!("agent-history-delete")) {
                disabled: $busy || active;
                tooltip: $crate::i18n::tr!("agent-history-delete-help");
                on_click: $cx.listener(move |this, _, _, cx| this.delete(target.clone(), cx));
            }
        });
        gds_uim::el! { #div {
            id: $format!("agent-history-actions-{key}"); test_support; flex; items_center; gap_2; flex_shrink_0;
        } [
            #icon_action($format!("agent-history-{suffix}-{key}"), icon, label) {
                tooltip: hint;
                disabled: $busy || !allowed; loading: loading;
                on_click: $cx.listener(move |this, _, _, cx| this.activate(primary_target.clone(), primary, cx));
            },
            ..close, ..delete,
        ] }.into_any_element()
    }

    fn record_status(
        &self,
        key: &str,
        agent: &str,
        info: &acp::SessionInfo,
        cx: &App,
    ) -> Option<AnyElement> {
        self.imported(agent, info).map(|session| {
            gds_uim::el! { #div { flex; items_center; flex_shrink_0; } [
                $session_status::Status::for_snapshot(session).render(
                    format!("agent-history-status-{key}"),
                    format!("agent-history-indicator-{key}"), cx,
                ),
                ..$session.meta.archived.then(|| session_status::archived(format!("agent-history-archived-{key}"))),
            ] }.into_any_element()
        })
    }

    fn row(
        &self,
        key: String,
        agent: &str,
        info: &acp::SessionInfo,
        page: &connection::AgentHistory,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = self.record_status(&key, agent, info, cx);
        let capabilities = &page.initialization.agent_capabilities;
        let resumable =
            capabilities.load_session || capabilities.session_capabilities.resume.is_some();
        let at = info.updated_at.as_deref().and_then(time::parse_rfc3339);
        let time = at
            .map(|at| time::recent_time(at, Timestamp::now()))
            .unwrap_or_else(|| crate::i18n::tr!("time-unknown").into());
        let raw_time = info
            .updated_at
            .clone()
            .unwrap_or_else(|| crate::i18n::tr!("time-unknown").into());
        let title = info
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(crate::i18n::tr!("agent-history-untitled"))
            .to_owned();
        let roots_warning = (!info.additional_directories.is_empty())
            .then(|| crate::i18n::tr!("agent-history-roots-unsupported"));
        let restore_warning = (!resumable && self.imported(agent, info).is_none())
            .then(|| crate::i18n::tr!("agent-history-cannot-import"));
        let actions = self.row_actions(key.clone(), agent, info, page, cx);
        gds_uim::el! { #div {
            id: $format!("agent-history-row-{key}"); test_support;
            flex; flex_col; gap_2; p_3; rounded_md; border_1; border_color: $cx.theme().border;
        } [
            #div { flex; items_center; gap_3; } [
                #div { id: $format!("agent-history-title-{key}"); test_support; role: gpui_kit::Role::Label; aria_label: $title.clone(); flex_1; min_w_0; truncate; } [$title],
                ..status,
                #div {
                    id: $format!("agent-history-time-{key}"); test_support;
                    text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                    hoverable_tooltip: $move |window, cx| crate::ui::tooltip::view(raw_time.clone(), window, cx);
                } [$time],
            ],
            ..$roots_warning.map(|message| crate::ui::feedback(
                format!("agent-history-roots-warning-{key}"), message, cx.theme().foreground,
            )),
            ..$restore_warning.map(|message| crate::ui::feedback(
                format!("agent-history-restore-warning-{key}"), message, cx.theme().foreground,
            )),
            #div { flex; items_center; gap_2; } [
                #div {
                    id: $format!("agent-history-directory-{key}"); test_support;
                    flex; items_center; gap_2; flex_1; min_w_0; text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [
                    Icon(IconName::Folder) { xsmall; flex_shrink_0; },
                    #div { min_w_0; truncate; } [$info.cwd.display().to_string()],
                ],
                $actions,
            ],
        ] }.into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.groups(cx);
        let empty =
            (groups.is_empty() && !self.loading() && self.connection_error.is_none()).then(|| {
                if self.agents.is_empty() {
                    crate::i18n::tr!("agent-history-no-agents")
                } else {
                    crate::i18n::tr!("agent-history-empty")
                }
            });
        let pending = self
            .operation
            .as_ref()
            .map(|_| crate::i18n::tr!("agent-history-operating"));
        gds_uim::el! { #div {
            id: "agent-history"; test_support; size_full; flex; text_size: $crate::ui::SMALL;
        } [
            $self.agent_filters(cx),
            #div { flex_1; min_w_0; min_h_0; flex; flex_col; p_4; gap_3; } [
                #div { flex; items_center; gap_2; } [
                    #div { flex_shrink_0; } [$crate::i18n::tr!("agent-history-workspace-filter")],
                    #div { flex_1; min_w_0; } [$self.workspace_filter(cx)],
                    #icon_action("agent-history-refresh", IconName::RefreshCw, $crate::i18n::tr!("agent-history-refresh")) {
                        disabled: $self.operation.is_some(); loading: $self.loading();
                        on_click: $cx.listener(|this, _, _, cx| this.reload(cx));
                    },
                ],
                $crate::ui::input::placeholder(&self.search, Input::new(&self.search).small().cleanable(true)),
                #div { text_color: $cx.theme().muted_foreground; } [$crate::i18n::tr!("agent-history-help")],
                ..$pending.map(|message| self.progress(
                    "agent-history-pending", message, cx,
                )),
                ..$self.connection_error.as_ref().map(|error| crate::ui::feedback(
                    "agent-history-connection-error", error.clone(), cx.theme().danger,
                )),
                ..$self.notice.clone().map(|message| crate::ui::feedback(
                    "agent-history-notice", message, cx.theme().foreground,
                )),
                #div { id: "agent-history-rows"; flex_1; min_h_0; overflow_y_scroll; } [
                    ..$self.loading().then(|| self.progress(
                        "agent-history-loading", crate::i18n::tr!("agent-history-loading"), cx,
                    )),
                    ..$empty.map(|message| crate::ui::feedback("agent-history-empty", message, cx.theme().foreground)),
                    ..groups,
                ],
            ],
        ] }
    }
}
