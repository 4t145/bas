//! Settings navigation is independent of the configuration pages it hosts.

use std::collections::HashSet;

use crate::ui::Button;
use gds_uim::el;
use gpui_kit::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, Subscription,
    TestSupportExt, Window,
    component::{button::*, input::*, *},
    div,
    prelude::*,
    px,
};

mod agents;
mod appearance;
#[cfg(all(test, feature = "gui-tests"))]
mod tests;
pub(crate) mod window;

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
        Button($id.clone()) {
            ghost;
            accessibility_label: $title.clone();
        } [
            #div {
                id: $format!("{id}-text");
                test_support;
                flex_1;
                min_w_0;
                text_left;
                truncate;
            } [$title],
        ]
    }
}

pub(crate) struct Close;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Category {
    Appearance,
    Agents,
}

impl Category {
    const ALL: [Self; 2] = [Self::Appearance, Self::Agents];

    fn id(self) -> &'static str {
        match self {
            Self::Appearance => "settings-appearance",
            Self::Agents => "settings-agents",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => crate::i18n::tr!("settings-appearance"),
            Self::Agents => crate::i18n::tr!("settings-agents"),
        }
    }

    fn for_query(query: &str) -> Self {
        let query = query.trim().to_lowercase();
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
    agents: Entity<agents::Agents>,
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
        let agents = cx.new(|cx| agents::Agents::new(crate::configuration::store(cx), window, cx));
        let appearance = cx.new(|cx| appearance::AppearancePage::new(editor, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, Self::on_search),
            cx.observe(&agents, Self::on_agents_changed),
            cx.observe_global_in::<crate::theme::Appearance>(window, Self::on_appearance_changed),
        ];
        Self {
            focus: cx.focus_handle(),
            search,
            appearance,
            agents,
            category: Category::Appearance,
            appearance_group: None,
            agent_section: None,
            expanded: Category::ALL.into_iter().collect(),
            _subscriptions: subscriptions,
        }
    }

    fn on_search(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let query = self.search.read(cx).value().to_string();
            if !query.trim().is_empty() {
                if self.category != Category::Agents {
                    self.category = Category::for_query(&query);
                }
                self.expanded.insert(self.category);
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

    fn on_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| {
            input.set_placeholder(crate::i18n::tr!("settings-search"), window, cx)
        });
        self.agents.update(cx, |page, cx| page.localize(window, cx));
        cx.notify();
    }

    fn on_close(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(Close);
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
            Category::Agents => self.select_agents(None, window, cx),
        }
    }

    pub(crate) fn flush_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance
            .update(cx, |page, cx| page.flush_pending(window, cx));
    }

    pub(crate) fn show_agent(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.select_agents(None, window, cx);
        self.agent_section = self.agents.update(cx, |page, cx| page.reveal(id, cx));
        cx.notify();
    }

    pub(crate) fn show_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_agents(None, window, cx);
        self.agents
            .update(cx, |page, cx| page.open_installation(cx));
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
            Category::Appearance => appearance::Group::ALL
                .into_iter()
                .map(|item| {
                    el! {
                        #navigation_button($item.id(), $item.title()) {
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
                        #navigation_button($section.id, $section.title) {
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
            Category::Agents => self.agent_section.is_some(),
        };
        let sections = self.section_navigation(category, selected, searching, cx);
        el! {
            #div {
                flex;
                flex_col;
                gap_1;
            } [
                #div {
                    flex;
                    items_center;
                } [
                    Button($format!("{}-expand", category.id())) {
                        ghost;
                        xsmall;
                        icon: $if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        };
                        accessibility_label: $format!(
                            "{}{}",
                            if expanded {
                                crate::i18n::tr!("action-collapse")
                            } else {
                                crate::i18n::tr!("action-expand")
                            },
                            category.title()
                        );
                        on_click: $cx.listener(move |this, _, _, cx| {
                            this.toggle_category(category, cx)
                        });
                    },
                    #navigation_button($category.id(), $category.title()) {
                        flex_1;
                        selected: $(selected && (!expanded || !pointing_at_section || searching));
                        on_click: $cx.listener(move |this, _, window, cx| {
                            this.select_category(category, window, cx)
                        });
                    },
                ],
            ].gds_uim::m! {
                @expanded: $|navigation| navigation.child(el! {
                    #div {
                        ml_3;
                        pl_5;
                        border_l_1;
                        border_color: $cx.theme().border;
                        flex;
                        flex_col;
                        gap_1;
                    } [..sections]
                });
            }
        }
        .into_any_element()
    }
}

impl EventEmitter<Close> for Settings {}
impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        el! {
            #div {
                id: "settings";
                track_focus: $&self.focus;
                size_full;
                flex;
                bg: $cx.theme().background;
                text_color: $cx.theme().foreground;
            } [
                #div {
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
                    #div {
                        id: "settings-search";
                        test_support;
                    } [Input($&self.search)],
                    #div {
                        id: "settings-navigation";
                        flex_1;
                        min_h_0;
                        overflow_y_scroll;
                        flex;
                        flex_col;
                        gap_2;
                    } [
                        ..$Category::ALL
                            .into_iter()
                            .map(|category| self.category_navigation(category, cx)),
                    ],
                    #div {
                        text_size: $crate::ui::SMALL;
                        text_color: $cx.theme().muted_foreground;
                    } [
                        $(match self.category {
                            Category::Appearance => crate::i18n::tr!("settings-appearance-footer"),
                            Category::Agents => crate::i18n::tr!("settings-agents-footer"),
                        }),
                    ],
                    #action_button("settings-close") {
                        label: $crate::i18n::tr!("settings-close");
                        on_click: $cx.listener(Self::on_close);
                    },
                ],
                #div {
                    flex_1;
                    min_w_0;
                    h_full;
                } [
                    $(match self.category {
                        Category::Appearance => self.appearance.clone().into_any_element(),
                        Category::Agents => self.agents.clone().into_any_element(),
                    }),
                ],
            ]
        }
    }
}
