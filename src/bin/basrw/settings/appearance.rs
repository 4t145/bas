//! Appearance owns its form and persistence; the settings shell owns navigation.

mod form;
use crate::theme::{self, preferences::Preferences};
use form::{DraftError, Form};
use gpui_kit::{
    App, Context, Render, ScrollHandle, Subscription, Task, TestSupportExt, Window,
    component::{group_box::*, input::*, select::*, *},
    div,
    prelude::*,
    px,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Theme,
    Fonts,
    Layout,
}
impl Group {
    pub const ALL: [Self; 3] = [Self::Theme, Self::Fonts, Self::Layout];
    pub fn title(self) -> &'static str {
        match self {
            Self::Theme => "主题",
            Self::Fonts => "字体",
            Self::Layout => "布局",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Theme => "主题来源、明暗模式与窗口背景。",
            Self::Fonts => "界面与正文使用的字体和字号。",
            Self::Layout => "正文行高与内容不透明度。",
        }
    }
    /// Navigation entry in the settings sidebar.
    pub fn id(self) -> &'static str {
        match self {
            Self::Theme => "settings-theme",
            Self::Fonts => "settings-fonts",
            Self::Layout => "settings-layout",
        }
    }
    /// Card that holds this group's rows in the page body.
    fn card_id(self) -> &'static str {
        match self {
            Self::Theme => "appearance-theme-card",
            Self::Fonts => "appearance-fonts-card",
            Self::Layout => "appearance-layout-card",
        }
    }
    pub(super) fn index(self) -> usize {
        match self {
            Self::Theme => 0,
            Self::Fonts => 1,
            Self::Layout => 2,
        }
    }
}

#[derive(Clone, Copy)]
enum Field {
    Source,
    Mode,
    Background,
    ThemeFile,
    UiFont,
    MonoFont,
    Fallbacks,
    UiSize,
    ContentSize,
    LineHeight,
    Opacity,
}
impl Field {
    const ALL: [Self; 11] = [
        Self::Source,
        Self::Mode,
        Self::Background,
        Self::ThemeFile,
        Self::UiFont,
        Self::MonoFont,
        Self::Fallbacks,
        Self::UiSize,
        Self::ContentSize,
        Self::LineHeight,
        Self::Opacity,
    ];
    fn id(self) -> &'static str {
        match self {
            Self::Source => "appearance-source",
            Self::Mode => "appearance-mode",
            Self::Background => "appearance-background",
            Self::ThemeFile => "appearance-theme-file",
            Self::UiFont => "appearance-ui-font",
            Self::MonoFont => "appearance-mono-font",
            Self::Fallbacks => "appearance-fallbacks",
            Self::UiSize => "appearance-ui-size",
            Self::ContentSize => "appearance-content-size",
            Self::LineHeight => "appearance-line-height",
            Self::Opacity => "appearance-opacity",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Source => "主题来源",
            Self::Mode => "主题模式",
            Self::Background => "窗口背景",
            Self::ThemeFile => "外部主题文件",
            Self::UiFont => "UI 字体",
            Self::MonoFont => "等宽字体",
            Self::Fallbacks => "回退字体",
            Self::UiSize => "UI 字号",
            Self::ContentSize => "正文字号",
            Self::LineHeight => "正文行高",
            Self::Opacity => "内容不透明度",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Self::Source => "Default 跟随桌面；内置不读取外部主题",
            Self::Mode => "选择浅色、深色或跟随桌面和系统",
            Self::Background => "Default 使用模糊背景",
            Self::ThemeFile => "相对配置目录或绝对路径；留空继承",
            Self::UiFont => "搜索已安装字体；Default 继承桌面或内置字体",
            Self::MonoFont => "用于代码；保留配置中的未安装字体",
            Self::Fallbacks => "逗号分隔；留空继承，[] 不指定额外回退",
            Self::UiSize | Self::ContentSize => "像素；留空继承",
            Self::LineHeight => "字号倍数；留空继承",
            Self::Opacity => "0–1；留空继承",
        }
    }
    fn group(self) -> Group {
        match self {
            Self::Source | Self::Mode | Self::Background | Self::ThemeFile => Group::Theme,
            Self::UiFont | Self::MonoFont | Self::Fallbacks | Self::UiSize | Self::ContentSize => {
                Group::Fonts
            }
            Self::LineHeight | Self::Opacity => Group::Layout,
        }
    }
    fn matches(self, query: &str) -> bool {
        format!(
            "外观 {} {} {}",
            self.group().title(),
            self.label(),
            self.help()
        )
        .to_lowercase()
        .contains(query)
    }
}

pub(super) struct AppearancePage {
    editor: theme::Editor,
    document: Option<theme::Document>,
    form: Form,
    pub(super) query: String,
    /// Section the sidebar asked to reveal; consumed by the next render.
    pub(super) scroll_to: Option<usize>,
    scroll: ScrollHandle,
    message: String,
    pending: Option<Task<()>>,
    reload: bool,
    subscriptions: Vec<Subscription>,
    _appearance: Subscription,
}

impl AppearancePage {
    pub(super) fn new(editor: theme::Editor, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (document, message) = match editor.load() {
            Ok(document) => (Some(document), String::new()),
            Err(error) => (None, format!("无法读取配置：{error:#}")),
        };
        let preferences = document
            .as_ref()
            .map(|d| d.preferences.clone())
            .unwrap_or_default();
        let form = Form::new(&preferences, window, cx);
        let mut page = Self {
            editor,
            document,
            form,
            query: String::new(),
            scroll_to: None,
            scroll: ScrollHandle::new(),
            message,
            pending: None,
            reload: false,
            subscriptions: Vec::new(),
            _appearance: cx.observe_global::<theme::Appearance>(|this, cx| {
                this.reload = true;
                cx.notify();
            }),
        };
        page.subscribe(window, cx);
        page
    }

    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.clear();
        for input in [
            &self.form.fallbacks,
            &self.form.ui_size,
            &self.form.content_size,
            &self.form.line_height,
            &self.form.opacity,
            &self.form.theme_file,
        ] {
            self.subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        if this.document.is_none() {
                            cx.notify();
                            return;
                        }
                        this.message = this
                            .candidate(cx)
                            .err()
                            .map(|error| format!("未应用：{error}"))
                            .unwrap_or_default();
                        this.pending = Some(cx.spawn_in(window, async move |view, cx| {
                            cx.background_executor().timer(DEBOUNCE).await;
                            let _ = view.update_in(cx, |this, window, cx| this.save(window, cx));
                        }));
                        cx.notify();
                    }
                },
            ));
        }
        macro_rules! select {
            ($field:ident) => {
                self.subscriptions.push(cx.subscribe_in(
                    &self.form.$field,
                    window,
                    |this, _, event, window, cx| {
                        if matches!(event, SelectEvent::Confirm(_)) {
                            this.flush_pending(window, cx);
                        }
                    },
                ));
            };
        }
        select!(source);
        select!(mode);
        select!(background);
        select!(ui_font);
        select!(mono_font);
    }

    /// # Errors
    /// Rejects malformed numbers and invalid appearance values without changing live state.
    fn candidate(&self, cx: &App) -> Result<Preferences, DraftError> {
        self.form.candidate(cx)
    }

    fn dirty(&self, cx: &App) -> bool {
        match (self.candidate(cx), &self.document) {
            (Ok(candidate), Some(document)) => candidate != document.preferences,
            (_, None) => false,
            _ => true,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty(cx) {
            return;
        }
        self.message = match self.candidate(cx) {
            Err(error) => format!("未应用：{error}"),
            Ok(candidate) => match self.document.as_mut() {
                None => "未应用：请修复配置文件后重新打开设置".into(),
                Some(document) => match self.editor.save(document, candidate, window, cx) {
                    Ok(()) => "已保存并应用".into(),
                    Err(error)
                        if matches!(
                            error.downcast_ref::<basr::config::Error>(),
                            Some(basr::config::Error::Conflict { .. })
                        ) =>
                    {
                        "未应用：外观配置已被外部修改；当前输入保留，请重新打开设置后再修改。"
                            .into()
                    }
                    Err(error) => format!("未应用：{error:#}"),
                },
            },
        };
        cx.notify();
    }

    pub(super) fn flush_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        self.save(window, cx);
    }

    fn replace_form(
        &mut self,
        preferences: &Preferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        self.form = Form::new(preferences, window, cx);
        self.subscribe(window, cx);
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload = false;
        if self.dirty(cx) {
            return; // Keep invalid/in-progress edits and their original conflict baseline.
        }
        match self.editor.load() {
            Ok(document) => {
                if self
                    .document
                    .as_ref()
                    .is_none_or(|old| old.preferences != document.preferences)
                {
                    self.replace_form(&document.preferences, window, cx);
                    self.message.clear();
                }
                self.document = Some(document);
            }
            Err(error) => self.message = format!("无法重载配置：{error:#}"),
        }
    }

    fn control(&self, field: Field, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match field {
            Field::Source => Select::new(&self.form.source).into_any_element(),
            Field::Mode => Select::new(&self.form.mode).into_any_element(),
            Field::Background => Select::new(&self.form.background).into_any_element(),
            Field::UiFont => Select::new(&self.form.ui_font)
                .search_placeholder("搜索字体")
                .into_any_element(),
            Field::MonoFont => Select::new(&self.form.mono_font)
                .search_placeholder("搜索字体")
                .into_any_element(),
            _ => {
                let input = match field {
                    Field::ThemeFile => &self.form.theme_file,
                    Field::Fallbacks => &self.form.fallbacks,
                    Field::UiSize => &self.form.ui_size,
                    Field::ContentSize => &self.form.content_size,
                    Field::LineHeight => &self.form.line_height,
                    Field::Opacity => &self.form.opacity,
                    _ => unreachable!("select fields handled above"),
                }
                .clone();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id(field.id())
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input)),
                    )
                    .child(
                        super::action_button(format!("{}-default", field.id()))
                            .label("Default")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                                this.flush_pending(window, cx);
                            })),
                    )
                    .into_any_element()
            }
        }
    }
    fn row(&self, field: Field, last: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .id(format!("{}-row", field.id()))
            .test_support()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_3()
            .py_3()
            .when(!last, |row| {
                row.border_b_1().border_color(cx.theme().border)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(field.label())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(field.help()),
                    ),
            )
            .child(
                div()
                    .w(px(250.))
                    .flex_shrink_0()
                    .child(self.control(field, cx)),
            )
            .into_any_element()
    }

    fn group_card(
        &self,
        group: Group,
        fields: &[Field],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let last = fields.len().saturating_sub(1);
        div()
            .id(group.card_id())
            .test_support()
            .child(
                GroupBox::new()
                    .outline()
                    .title(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .text_color(cx.theme().foreground)
                                    .child(group.title()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(group.help()),
                            ),
                    )
                    .children(
                        fields
                            .iter()
                            .enumerate()
                            .map(|(index, field)| self.row(*field, index == last, cx)),
                    ),
            )
            .into_any_element()
    }

    /// Sections rendered for the current search, in page order.
    fn sections(&self) -> Vec<(Group, Vec<Field>)> {
        let query = self.query.trim().to_lowercase();
        Group::ALL
            .into_iter()
            .filter_map(|group| {
                let fields: Vec<_> = Field::ALL
                    .into_iter()
                    .filter(|field| field.group() == group)
                    .filter(|field| query.is_empty() || field.matches(&query))
                    .collect();
                (!fields.is_empty()).then_some((group, fields))
            })
            .collect()
    }
}

impl Render for AppearancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.reload {
            self.refresh(window, cx);
        }
        let sections = self.sections();
        if let Some(index) = self.scroll_to.take() {
            self.scroll
                .scroll_to_top_of_item(index.min(sections.len().saturating_sub(1)));
        }
        let mut header = div()
            .p_5()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xl().child("外观"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "当前主题：{}",
                        cx.global::<theme::Appearance>().name
                    )),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("更改自动保存并立即生效。Default 使用桌面或内置默认值。"),
            );
        if let Some(path) = self.editor.theme_override() {
            header = header.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("CLI --theme-file 优先：{}", path.display())),
            );
        }
        let body = div()
            .id("appearance-sections")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p_5()
            .flex()
            .flex_col()
            .gap_5()
            .when(sections.is_empty(), |body| {
                body.child(
                    div()
                        .id("settings-no-results")
                        .test_support()
                        .child("没有匹配的设置。尝试其他名称或描述。"),
                )
            })
            .children(
                sections
                    .iter()
                    .map(|(group, fields)| self.group_card(*group, fields, cx)),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .child(
                div()
                    .p_3()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().child(self.message.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_xs()
                                    .truncate()
                                    .child(format!("配置：{}", self.editor.path().display())),
                            )
                            .child(
                                super::action_button("settings-reset")
                                    .label("恢复全部默认")
                                    .disabled(self.document.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.replace_form(&Preferences::default(), window, cx);
                                        this.save(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
