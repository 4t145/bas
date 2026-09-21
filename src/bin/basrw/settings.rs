//! Settings navigation is independent of the configuration pages it hosts.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, Subscription,
    TestSupportExt, Window,
    component::{button::*, input::*, *},
    div,
    prelude::*,
    px,
};

mod appearance;
#[cfg(all(test, feature = "gui-tests"))]
mod tests;

// Keep actions distinct from the ghost navigation items, using upstream state styling.
fn action_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    Button::new(id).with_variant(ButtonVariant::Default).small()
}

pub(crate) struct Close;

pub(crate) struct Settings {
    focus: FocusHandle,
    search: Entity<InputState>,
    appearance: Entity<appearance::AppearancePage>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置…"));
        let editor = crate::theme::Editor::current(cx);
        let appearance = cx.new(|cx| appearance::AppearancePage::new(editor, window, cx));
        let subscriptions = vec![cx.subscribe_in(&search, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = this.search.read(cx).value().to_string();
                this.appearance.update(cx, |page, cx| {
                    page.query = query;
                    cx.notify();
                });
            }
        })];
        Self {
            focus: cx.focus_handle(),
            search,
            appearance,
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
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.appearance.update(cx, |page, cx| {
            page.group = group;
            page.query.clear();
            cx.notify();
        });
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
                        Button::new("settings-appearance")
                            .ghost()
                            .label("外观")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_appearance(None, window, cx)
                            })),
                    )
                    .children(appearance::Group::ALL.into_iter().map(|group| {
                        Button::new(group.id())
                            .ghost()
                            .label(group.title())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_appearance(Some(group), window, cx);
                            }))
                    }))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("有效修改自动保存"),
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
                    .child(self.appearance.clone()),
            )
    }
}
