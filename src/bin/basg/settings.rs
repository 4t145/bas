//! Settings navigation is independent of the configuration pages it hosts.

use std::collections::HashSet;

use crate::{
    i18n::tr,
    ui::{self, Button},
};
use gds_uim::el;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, Render, Subscription, TestSupportExt,
    Window,
    component::{button::*, input::*, *},
    div,
    prelude::*,
    px,
};

mod acp;
mod agents;
mod appearance;
mod connections;
mod form;
pub(crate) mod window;

const SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

/// Settings feedback is informational; copying remains available on other UI surfaces.
fn feedback(
    id: impl Into<gpui_kit::SharedString>,
    text: impl Into<gpui_kit::SharedString>,
    color: gpui_kit::Hsla,
) -> crate::ui::Feedback {
    crate::ui::feedback(id, text, color).without_copy()
}

// Keep actions distinct from the ghost navigation items, using upstream state styling.
fn action_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    crate::ui::action_button(id)
}

fn navigation_button(
    id: impl Into<gpui_kit::SharedString>,
    title: impl Into<gpui_kit::SharedString>,
) -> Button {
    let id = id.into();
    let title = title.into();
    // Button's label container centers its contents independently of the root.
    el! {
        (~id).Button::new {
            ghost;
            small;
            accessibility_label: ~title;
        } [
            .div {
                # format!("{id}-text");
                test_support;
                flex_1;
                min_w_0;
                text_left;
                truncate;
            } [title],
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Category {
    Appearance,
    Sessions,
    Connections,
    Agents,
    SessionManagement,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Frontend,
    Backend,
}

impl Scope {
    const ALL: [Self; 2] = [Self::Frontend, Self::Backend];

    fn categories(self) -> &'static [Category] {
        match self {
            Self::Frontend => &[
                Category::Appearance,
                Category::Sessions,
                Category::Connections,
            ],
            Self::Backend => &[Category::SessionManagement, Category::Agents],
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Frontend => crate::i18n::tr!("settings-scope-frontend"),
            Self::Backend => crate::i18n::tr!("settings-scope-backend"),
        }
    }
}

impl Category {
    fn scope(self) -> Scope {
        match self {
            Self::Appearance | Self::Sessions | Self::Connections => Scope::Frontend,
            Self::SessionManagement | Self::Agents => Scope::Backend,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Appearance => "settings-appearance",
            Self::Sessions => "settings-sessions",
            Self::Connections => "settings-connections",
            Self::Agents => "settings-agents",
            Self::SessionManagement => "settings-session-management",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => crate::i18n::tr!("settings-appearance"),
            Self::Sessions => crate::i18n::tr!("settings-sessions"),
            Self::Connections => crate::i18n::tr!("settings-connections"),
            Self::Agents => crate::i18n::tr!("settings-agents"),
            Self::SessionManagement => crate::i18n::tr!("settings-session-management"),
        }
    }

    fn for_query(query: &str) -> Self {
        let query = query.trim().to_lowercase();
        if query == "acp"
            || [
                "session management",
                "会话管理",
                "pool",
                "idle",
                "池",
                "回收",
                "预热",
            ]
            .iter()
            .any(|word| query.contains(word))
        {
            return Self::SessionManagement;
        }
        if !query.is_empty() && appearance::AppearancePage::matches_session_query(&query) {
            return Self::Sessions;
        }
        if ["connection", "server", "连接", "服务器", "端点"]
            .iter()
            .any(|word| query.contains(word))
        {
            return Self::Connections;
        }
        let agents = query.split_whitespace().any(|word| {
            matches!(
                word,
                "agent" | "agents" | "acp" | "codex" | "opencode" | "pi"
            )
        }) || [
            "智能体",
            "服务",
            "后端",
            "命令",
            "参数",
            "service",
            "backend",
            "command",
            "arguments",
        ]
        .iter()
        .any(|word| query.contains(word));
        if agents {
            Self::Agents
        } else {
            Self::Appearance
        }
    }
}

pub(crate) struct Settings {
    focus: FocusHandle,
    search: Entity<InputState>,
    appearance: Entity<appearance::AppearancePage>,
    connections: Entity<connections::Page>,
    profiles: Entity<crate::connections::Profiles>,
    agents: Entity<agents::Agents>,
    _agents_subscription: Subscription,
    acp: Entity<acp::Page>,
    _acp_subscription: Subscription,
    connection_message: String,
    category: Category,
    /// Section the sidebar points at inside each page.
    appearance_group: Option<appearance::Group>,
    agent_section: Option<usize>,
    expanded: HashSet<Category>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder(crate::i18n::tr!("settings-search")));
        let editor = crate::theme::Editor::current(cx);
        let agents = agents::shared(window, cx);
        let appearance = cx.new(|cx| appearance::AppearancePage::new(editor, window, cx));
        let profiles = crate::connections::shared(cx);
        let connections = cx.new(|cx| connections::Page::new(window, cx));
        let acp = acp::shared(window, cx);
        let acp_subscription = cx.observe(&acp, |_, _, cx| cx.notify());
        let agents_subscription = cx.observe(&agents, Self::on_agents_changed);
        let subscriptions = vec![
            cx.subscribe_in(&search, window, Self::on_search),
            cx.observe(&appearance, |_, _, cx| cx.notify()),
            cx.observe(&connections, |_, _, cx| cx.notify()),
            cx.observe(&profiles, |_, _, cx| cx.notify()),
            cx.observe_global_in::<crate::agents::catalog::Service>(
                window,
                Self::on_backend_changed,
            ),
            cx.observe_global_in::<crate::theme::Appearance>(window, Self::on_appearance_changed),
        ];
        Self {
            focus: cx.focus_handle(),
            search,
            appearance,
            connections,
            profiles,
            agents,
            _agents_subscription: agents_subscription,
            acp,
            _acp_subscription: acp_subscription,
            connection_message: String::new(),
            category: Category::Appearance,
            appearance_group: None,
            agent_section: None,
            expanded: Scope::ALL
                .into_iter()
                .flat_map(|scope| scope.categories().iter().copied())
                .collect(),
            _subscriptions: subscriptions,
        }
    }

    fn on_search(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let query = self.search.read(cx).value().to_string();
            if !query.trim().is_empty() {
                let category = Category::for_query(&query);
                if self.category != Category::Agents
                    || matches!(
                        category,
                        Category::Sessions | Category::Connections | Category::SessionManagement
                    )
                {
                    self.category = category;
                }
                self.expanded.insert(self.category);
                if self.category == Category::SessionManagement {
                    self.acp
                        .update(cx, |page, cx| page.ensure_loaded(window, cx));
                }
            }
            self.appearance.update(cx, |page, cx| {
                page.query = query.clone();
                cx.notify();
            });
            self.agents.update(cx, |page, cx| {
                page.query = query;
                cx.notify();
            });
            cx.notify();
        }
    }

    fn on_agents_changed(&mut self, _: Entity<agents::Agents>, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn on_backend_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let agents = agents::shared(window, cx);
        if agents.entity_id() == self.agents.entity_id() {
            return;
        }
        self.acp.update(cx, |page, cx| page.prepare_to_leave(cx));
        self.agents = agents;
        self._agents_subscription = cx.observe(&self.agents, Self::on_agents_changed);
        self.acp = acp::shared(window, cx);
        self._acp_subscription = cx.observe(&self.acp, |_, _, cx| cx.notify());
        if self.category == Category::SessionManagement {
            self.acp.update(cx, |page, cx| page.open(window, cx));
        }
        self.agent_section = None;
        cx.notify();
    }

    fn choose_backend(&mut self, id: &str, cx: &mut Context<Self>) {
        match self
            .profiles
            .update(cx, |profiles, cx| profiles.select(id, cx))
        {
            Ok(target) => {
                self.connection_message.clear();
                crate::connections::manage(target, cx);
            }
            Err(error) => self.connection_message = error,
        }
        cx.notify();
    }

    fn on_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| {
            input.set_placeholder(crate::i18n::tr!("settings-search"), window, cx)
        });
        self.agents.update(cx, |page, cx| page.localize(window, cx));
        cx.notify();
    }

    fn toggle_category(&mut self, category: Category, cx: &mut Context<Self>) {
        if !self.expanded.remove(&category) {
            self.expanded.insert(category);
        }
        cx.notify();
    }

    fn select_category(&mut self, category: Category, window: &mut Window, cx: &mut Context<Self>) {
        match category {
            Category::Appearance => self.select_appearance(None, window, cx),
            Category::Sessions => {
                self.category = Category::Sessions;
                self.search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.appearance.update(cx, |page, cx| {
                    page.query.clear();
                    cx.notify();
                });
                cx.notify();
            }
            Category::Agents => self.select_agents(None, window, cx),
            Category::SessionManagement => {
                self.category = Category::SessionManagement;
                self.search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.acp.update(cx, |page, cx| page.open(window, cx));
                cx.notify();
            }
            Category::Connections => {
                self.category = Category::Connections;
                self.search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.connections
                    .update(cx, |page, cx| page.focus_page(window, cx));
                cx.notify();
            }
        }
    }

    pub(crate) fn flush_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance
            .update(cx, |page, cx| page.flush_pending(window, cx));
        self.acp.update(cx, |page, cx| page.flush_pending(cx));
    }

    fn prepare_to_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance
            .update(cx, |page, cx| page.flush_pending(window, cx));
        self.acp.update(cx, |page, cx| page.prepare_to_leave(cx));
    }

    pub(crate) fn show_agent(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.select_agents(None, window, cx);
        self.agent_section = self.agents.update(cx, |page, cx| page.reveal(id, cx));
        cx.notify();
    }

    fn settings_path(&self, cx: &App) -> Option<std::path::PathBuf> {
        match self.category {
            Category::Appearance | Category::Sessions => Some(
                crate::theme::Editor::current(cx)
                    .appearance_path()
                    .to_owned(),
            ),
            Category::Agents | Category::SessionManagement => {
                self.agents.read(cx).settings_path().map(ToOwned::to_owned)
            }
            Category::Connections => Some(crate::configuration::store(cx).path().to_owned()),
        }
    }

    fn can_edit_file(&self, cx: &App) -> bool {
        self.settings_path(cx).is_some_and(|path| path.is_file())
    }

    fn on_edit_file(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending(window, cx);
        if let Some(path) = self.settings_path(cx) {
            if self.category.scope() == Scope::Backend
                && let Some(write) = self.acp.read(cx).pending_write()
            {
                cx.spawn(async move |_, cx| {
                    write.await;
                    cx.update(|cx| cx.open_with_system(&path));
                })
                .detach();
                return;
            }
            cx.open_with_system(&path);
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.settings_path(cx);
        let file_hint = path.as_ref().map_or_else(
            || crate::i18n::tr!("settings-backend-file-unavailable").to_owned(),
            |path| {
                if path.is_file() {
                    path.display().to_string()
                } else {
                    crate::i18n::tr!("settings-file-not-created").to_owned()
                }
            },
        );
        let context_id = match self.category.scope() {
            Scope::Frontend => "settings-frontend-context",
            Scope::Backend => "settings-backend-context",
        };
        let context = match self.category.scope() {
            Scope::Frontend => crate::i18n::tr!("settings-frontend-context").to_owned(),
            Scope::Backend => {
                let backend = cx
                    .try_global::<crate::agents::catalog::Service>()
                    .map_or_else(
                        || crate::i18n::tr!("settings-backend-disconnected").to_owned(),
                        |service| service.address().to_string(),
                    );
                crate::i18n::tr!("settings-backend-context", address = backend)
            }
        };
        el! {
            .div {
                # "settings-header"; test_support;
                px_5; py_3; flex_shrink_0; flex; flex_col; gap_2;
                border_b_1; border_color: $cx.theme().border;
            } [
                .div { flex; items_center; justify_between; gap_3; } [
                    .div {
                        # context_id; test_support;
                        role: gpui_kit::Role::Label;
                        aria_label: ~context;
                        flex_1; min_w_0; truncate;
                    } [context],
                    @if (self.category.scope() == Scope::Backend)
                        $self.backend_selector(cx),
                    @end,
                    .div {
                        # "settings-actions"; test_support;
                        flex; items_center; gap_2; flex_shrink_0;
                    } [
                        ("settings-edit-file").Button::new {
                            ghost; small; flex_shrink_0;
                            label: tr!("settings-edit-file");
                            disabled: $!self.can_edit_file(cx);
                            tooltip: file_hint;
                            on_click: (cx, Self::on_edit_file).Context::listener;
                        },
                    ],
                ],
                .div {
                    # "settings-scope-description"; test_support;
                    text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [
                    @match (self.category)
                    @case (Category::Appearance | Category::Sessions)
                        tr!("settings-frontend-description") { to_owned },
                    @case (Category::Connections)
                        tr!("connections-description") { to_owned },
                    @case (Category::Agents | Category::SessionManagement)
                        tr!("connections-switch-description") { to_owned },
                    @end,
                ],
            ]
        }
    }

    fn backend_selector(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        use crate::connections::presentation;
        let profiles = self.profiles.read(cx).entries().to_vec();
        let active = crate::connections::current(cx);
        let selected = active
            .as_ref()
            .and_then(|options| self.profiles.read(cx).matching(options))
            .cloned();
        let label = selected
            .as_ref()
            .map(|profile| profile.name.clone())
            .or_else(|| {
                active
                    .as_ref()
                    .map(|options| presentation::endpoint(&options.target))
            })
            .unwrap_or_else(|| crate::i18n::tr!("connections-choose").into());
        let hint = active.as_ref().map_or_else(
            || label.clone(),
            |options| format!("{label} · {}", presentation::tooltip(&options.target)),
        );
        let owner = cx.entity().downgrade();
        let button = el! { ("settings-backend-selector").Button::new {
            small; ghost; min_w_0; max_w: 200.0.px;
            accessibility_label: ~hint;
            tooltip: hint;
        } [
            @if (let Some(options) = active.as_ref())
                ($&options.target).presentation::icon { size: ui::ICON.px },
            @end,
            .div { min_w_0; truncate; } [label],
            (IconName::ChevronDown).Icon::new { xsmall; },
        ] };
        el! { button {
            dropdown_menu: $move |menu, _, _| {
                let mut menu = ui::menu(menu).check_side(gpui_kit::component::Side::Right);
                for profile in &profiles {
                    let owner = owner.clone();
                    let id = profile.id.clone();
                    menu = menu.item(presentation::menu_item(profile)
                        .checked(selected.as_ref().is_some_and(|selected| selected.id == profile.id))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.choose_backend(&id, cx));
                        }));
                }
                let owner = owner.clone();
                menu.separator().item(PopupMenuItem::new(tr!("connections-manage"))
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| this.select_category(Category::Connections, window, cx));
                    }))
            };
        } }.into_any_element()
    }

    /// Point the appearance page at one of its sections without hiding the others.
    fn select_appearance(
        &mut self,
        group: Option<appearance::Group>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.category = Category::Appearance;
        self.expanded.insert(Category::Appearance);
        self.appearance_group = group;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        let target = group.map_or(0, appearance::Group::index);
        self.appearance.update(cx, |page, cx| {
            page.scroll_to = Some(target);
            page.query.clear();
            cx.notify();
        });
        cx.notify();
    }

    /// Point the agents page at one agent section; `None` selects the page top.
    fn select_agents(
        &mut self,
        section: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.category = Category::Agents;
        self.expanded.insert(Category::Agents);
        self.agent_section = section;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.agents.update(cx, |page, cx| {
            page.show_section(section, cx);
        });
        cx.notify();
    }

    /// Sidebar entries below a category, in the order the page renders them.
    fn section_navigation(
        &self,
        category: Category,
        selected: bool,
        searching: bool,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        match category {
            Category::Sessions | Category::Connections | Category::SessionManagement => Vec::new(),
            Category::Appearance => appearance::Group::ALL
                .into_iter()
                .map(|item| {
                    el! {
                        ($item.id(), $item.title()).navigation_button {
                            selected: $(selected && !searching && self.appearance_group == Some(item));
                            on_click: $cx.listener(move |this, _, window, cx| {
                                this.select_appearance(Some(item), window, cx)
                            });
                        }
                    }
                    .into_any_element()
                })
                .collect(),
            Category::Agents => self
                .agents
                .read(cx)
                .sections()
                .into_iter()
                .enumerate()
                .map(|(index, section)| {
                    el! {
                        ($section.id, $section.title).navigation_button {
                            selected: $(selected && !searching && self.agent_section == Some(index));
                            on_click: $cx.listener(move |this, _, window, cx| {
                                this.select_agents(Some(index), window, cx)
                            });
                        }
                    }
                    .into_any_element()
                })
                .collect(),
        }
    }

    fn category_navigation(
        &self,
        category: Category,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let expanded = self.expanded.contains(&category);
        let searching = !self.search.read(cx).value().is_empty();
        let selected = self.category == category;
        let pointing_at_section = match category {
            Category::Appearance => self.appearance_group.is_some(),
            Category::Sessions | Category::Connections | Category::SessionManagement => false,
            Category::Agents => self.agent_section.is_some(),
        };
        let sections = self.section_navigation(category, selected, searching, cx);
        if matches!(
            category,
            Category::Sessions | Category::Connections | Category::SessionManagement
        ) {
            return el! { .div { pl: ui::COMPACT_CONTROL.px; flex; } [
                ($category.id(), $category.title()).navigation_button {
                    flex_1; selected: selected;
                    on_click: $cx.listener(move |this, _, window, cx| {
                        this.select_category(category, window, cx)
                    });
                },
            ] }
            .into_any_element();
        }
        el! {
            .div {
                flex;
                flex_col;
                gap_1;
            } [
                .div {
                    flex;
                    items_center;
                } [
                    (format!("{}-expand", category.id())).Button::new {
                        ghost;
                        xsmall;
                        w: ui::COMPACT_CONTROL.px;
                        h: ui::COMPACT_CONTROL.px;
                        p_0;
                        icon: @if (expanded)
                            IconName::ChevronDown
                        @else
                            IconName::ChevronRight
                        @end;
                        accessibility_label: format!(
                            "{}{}",
                            if expanded {
                                tr!("action-collapse")
                            } else {
                                tr!("action-expand")
                            },
                            category.title()
                        );
                        on_click: $cx.listener(move |this, _, _, cx| {
                            this.toggle_category(category, cx)
                        });
                    },
                    ($category.id(), $category.title()).navigation_button {
                        flex_1;
                        selected: $(selected && (!expanded || !pointing_at_section || searching));
                        on_click: $cx.listener(move |this, _, window, cx| {
                            this.select_category(category, window, cx)
                        });
                    },
                ],
                @if (expanded)
                    .div {
                        ml_3;
                        pl_5;
                        border_l_1;
                        border_color: $cx.theme().border;
                        flex;
                        flex_col;
                        gap_1;
                    } [..sections],
                @end,
            ]
        }
        .into_any_element()
    }
}

impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feedback = match self.category {
            Category::Appearance | Category::Sessions => self.appearance.read(cx).message.clone(),
            Category::Agents => self.agents.read(cx).save_status().unwrap_or_default(),
            Category::SessionManagement => self.acp.read(cx).status(cx),
            Category::Connections => self.connections.read(cx).status(cx),
        };
        let feedback = if self.connection_message.is_empty() {
            feedback
        } else {
            self.connection_message.clone()
        };
        el! {
            .div {
                # "settings";
                track_focus: $&self.focus;
                size_full;
                flex;
                bg: $cx.theme().background;
                text_color: $cx.theme().foreground;
            } [
                .div {
                    w: 210.0.px;
                    flex_shrink_0;
                    p_3;
                    flex;
                    flex_col;
                    gap_2;
                    bg: $cx.theme().sidebar;
                    border_r_1;
                    border_color: $cx.theme().border;
                } [
                    .div {
                        # "settings-search";
                        test_support;
                    } [$ui::input::placeholder(&self.search, Input::new(&self.search).small())],
                    .div {
                        # "settings-navigation";
                        test_support;
                        flex_1;
                        min_h_0;
                        overflow_y_scroll;
                        flex;
                        flex_col;
                        gap_2;
                    } [
                        ..Scope::ALL {
                            into_iter;
                            map: scope =>
                                .div {
                                    # @if (scope == Scope::Frontend) "settings-frontend-group" @else "settings-backend-group" @end;
                                    test_support; flex; flex_col; gap_1;
                                } [
                                    .div {
                                        mt_2; text_size: ui::SMALL;
                                        text_color: $cx.theme().muted_foreground;
                                    } [$scope.title()],
                                    ..$scope.categories().iter().map(|category| self.category_navigation(*category, cx)),
                                ]
                        },
                    ],
                    ("settings-save-status", feedback, $cx.theme().muted_foreground).self::feedback { compact },
                ],
                .div {
                    flex_1;
                    min_w_0;
                    h_full;
                    flex; flex_col;
                } [
                    $self.header(cx),
                    .div { flex_1; min_h_0; } [
                        @match (self.category)
                        @case (Category::Appearance)
                            ~$(self.appearance),
                        @case (Category::Sessions)
                            $self.appearance.update(cx, |page, cx| page.render_sessions(window, cx)),
                        @case (Category::Agents)
                            ~$(self.agents),
                        @case (Category::SessionManagement)
                            ~$(self.acp),
                        @case (Category::Connections)
                            ~$(self.connections),
                        @end,
                    ],
                ],
            ]
        }
    }
}
