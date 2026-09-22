//! Settings navigation is independent of the configuration pages it hosts.

use std::collections::HashSet;

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, Subscription,
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

// Keep actions distinct from the ghost navigation items, using upstream state styling.
fn action_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    Button::new(id).with_variant(ButtonVariant::Default).small()
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
            Self::Appearance => "外观",
            Self::Agents => "Agent 服务",
        }
    }

    fn for_query(query: &str) -> Self {
        let query = query.trim().to_lowercase();
        let agents = query.split_whitespace().any(|word| {
            matches!(
                word,
                "agent" | "agents" | "acp" | "codex" | "opencode" | "pi"
            )
        }) || ["服务", "后端", "命令", "参数"]
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
    expanded: HashSet<Category>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置…"));
        let editor = crate::theme::Editor::current(cx);
        let agents = cx.new(|cx| agents::Agents::new(editor.store(), window, cx));
        let appearance = cx.new(|cx| appearance::AppearancePage::new(editor, window, cx));
        let subscriptions = vec![cx.subscribe_in(&search, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = this.search.read(cx).value().to_string();
                if !query.trim().is_empty() {
                    this.category = Category::for_query(&query);
                    this.expanded.insert(this.category);
                }
                this.appearance.update(cx, |page, cx| {
                    page.query = query;
                    cx.notify();
                });
                cx.notify();
            }
        })];
        Self {
            focus: cx.focus_handle(),
            search,
            appearance,
            agents,
            category: Category::Appearance,
            expanded: Category::ALL.into_iter().collect(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn flush_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance
            .update(cx, |page, cx| page.flush_pending(window, cx));
    }

    fn select_appearance(
        &mut self,
        group: Option<appearance::Group>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.category = Category::Appearance;
        self.expanded.insert(Category::Appearance);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.appearance.update(cx, |page, cx| {
            page.group = group;
            page.query.clear();
            cx.notify();
        });
        cx.notify();
    }

    fn select_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.category = Category::Agents;
        self.expanded.insert(Category::Agents);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn category_navigation(
        &self,
        category: Category,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let expanded = self.expanded.contains(&category);
        let group = self.appearance.read(cx).group;
        let searching = !self.search.read(cx).value().is_empty();
        let selected = self.category == category;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        Button::new(format!("{}-expand", category.id()))
                            .ghost()
                            .xsmall()
                            .icon(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .accessibility_label(format!(
                                "{}{}",
                                if expanded { "收起" } else { "展开" },
                                category.title()
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&category) {
                                    this.expanded.insert(category);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(category.id())
                            .ghost()
                            .flex_1()
                            .justify_start()
                            .label(category.title())
                            .selected(
                                selected
                                    && (!expanded
                                        || (category == Category::Appearance
                                            && (group.is_none() || searching))),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| match category {
                                Category::Appearance => this.select_appearance(None, window, cx),
                                Category::Agents => this.select_agents(window, cx),
                            })),
                    ),
            )
            .when(expanded, |navigation| {
                navigation.child(
                    div()
                        .ml_3()
                        .pl_5()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(category == Category::Appearance, |children| {
                            children.children(appearance::Group::ALL.into_iter().map(|item| {
                                Button::new(item.id())
                                    .ghost()
                                    .justify_start()
                                    .label(item.title())
                                    .selected(selected && !searching && group == Some(item))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_appearance(Some(item), window, cx)
                                    }))
                            }))
                        })
                        .when(category == Category::Agents, |children| {
                            children.child(
                                Button::new("settings-acp-agents")
                                    .ghost()
                                    .justify_start()
                                    .label("ACP agents")
                                    .selected(selected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select_agents(window, cx)
                                    })),
                            )
                        }),
                )
            })
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
        div()
            .id("settings")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .w(px(210.))
                    .flex_shrink_0()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .id("settings-search")
                            .test_support()
                            .child(Input::new(&self.search)),
                    )
                    .child(
                        div()
                            .id("settings-navigation")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(
                                Category::ALL
                                    .into_iter()
                                    .map(|category| self.category_navigation(category, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match self.category {
                                Category::Appearance => "有效外观修改自动保存",
                                Category::Agents => "Agent 命令需明确保存",
                            }),
                    )
                    .child(
                        action_button("settings-close")
                            .label("关闭设置")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Close))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(match self.category {
                        Category::Appearance => self.appearance.clone().into_any_element(),
                        Category::Agents => self.agents.clone().into_any_element(),
                    }),
            )
    }
}
