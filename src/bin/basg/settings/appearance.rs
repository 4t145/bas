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
            Self::Theme => crate::i18n::tr!("appearance-theme"),
            Self::Fonts => crate::i18n::tr!("appearance-fonts"),
            Self::Layout => crate::i18n::tr!("appearance-layout"),
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Theme => crate::i18n::tr!("appearance-theme-help"),
            Self::Fonts => crate::i18n::tr!("appearance-fonts-help"),
            Self::Layout => crate::i18n::tr!("appearance-layout-help"),
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
    Language,
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
    ComposerEnter,
}
impl Field {
    const ALL: [Self; 13] = [
        Self::Language,
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
        Self::ComposerEnter,
    ];
    fn id(self) -> &'static str {
        match self {
            Self::Language => "appearance-language",
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
            Self::ComposerEnter => "appearance-composer-enter",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Language => crate::i18n::tr!("appearance-language"),
            Self::Source => crate::i18n::tr!("appearance-source"),
            Self::Mode => crate::i18n::tr!("appearance-mode"),
            Self::Background => crate::i18n::tr!("appearance-background"),
            Self::ThemeFile => crate::i18n::tr!("appearance-theme-file"),
            Self::UiFont => crate::i18n::tr!("appearance-ui-font"),
            Self::MonoFont => crate::i18n::tr!("appearance-mono-font"),
            Self::Fallbacks => crate::i18n::tr!("appearance-fallbacks"),
            Self::UiSize => crate::i18n::tr!("appearance-ui-size"),
            Self::ContentSize => crate::i18n::tr!("appearance-content-size"),
            Self::LineHeight => crate::i18n::tr!("appearance-line-height"),
            Self::Opacity => crate::i18n::tr!("appearance-opacity"),
            Self::ComposerEnter => crate::i18n::tr!("appearance-enter"),
        }
    }
    fn help(self) -> &'static str {
        match self {
            Self::Language => crate::i18n::tr!("appearance-language-help"),
            Self::Source => crate::i18n::tr!("appearance-source-help"),
            Self::Mode => crate::i18n::tr!("appearance-mode-help"),
            Self::Background => crate::i18n::tr!("appearance-background-help"),
            Self::ThemeFile => crate::i18n::tr!("appearance-theme-file-help"),
            Self::UiFont => crate::i18n::tr!("appearance-ui-font-help"),
            Self::MonoFont => crate::i18n::tr!("appearance-mono-font-help"),
            Self::Fallbacks => crate::i18n::tr!("appearance-fallbacks-help"),
            Self::UiSize | Self::ContentSize => crate::i18n::tr!("appearance-size-help"),
            Self::LineHeight => crate::i18n::tr!("appearance-line-height-help"),
            Self::Opacity => crate::i18n::tr!("appearance-opacity-help"),
            Self::ComposerEnter => {
                crate::i18n::tr!("appearance-enter-help")
            }
        }
    }
    fn group(self) -> Group {
        match self {
            Self::Language => Group::Theme,
            Self::Source | Self::Mode | Self::Background | Self::ThemeFile => Group::Theme,
            Self::UiFont | Self::MonoFont | Self::Fallbacks | Self::UiSize | Self::ContentSize => {
                Group::Fonts
            }
            Self::LineHeight | Self::Opacity | Self::ComposerEnter => Group::Layout,
        }
    }
    fn matches(self, query: &str) -> bool {
        format!(
            "{} {} {} {}",
            crate::i18n::tr!("settings-appearance"),
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
            Err(error) => (
                None,
                crate::i18n::tr!("settings-load-error", error = format!("{:#}", error)),
            ),
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
            _appearance: cx.observe_global_in::<theme::Appearance>(window, |this, window, cx| {
                for input in [
                    &this.form.fallbacks,
                    &this.form.ui_size,
                    &this.form.content_size,
                    &this.form.line_height,
                    &this.form.opacity,
                    &this.form.theme_file,
                ] {
                    input.update(cx, |input, cx| {
                        input.set_placeholder(
                            crate::i18n::tr!("appearance-inherit-placeholder"),
                            window,
                            cx,
                        )
                    });
                }
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
                            .map(|error| {
                                crate::i18n::tr!(
                                    "settings-apply-error",
                                    error = (error).to_string()
                                )
                            })
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
        select!(language);
        select!(mode);
        select!(background);
        select!(ui_font);
        select!(mono_font);
        select!(composer_enter);
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
            Err(error) => crate::i18n::tr!("settings-apply-error", error = (error).to_string()),
            Ok(candidate) => match self.document.as_mut() {
                None => crate::i18n::tr!("settings-fix-file").into(),
                Some(document) => match self.editor.save(document, candidate, window, cx) {
                    Ok(()) => crate::i18n::tr!("settings-applied").into(),
                    Err(error)
                        if matches!(
                            error.downcast_ref::<bas::config::Error>(),
                            Some(bas::config::Error::Conflict { .. })
                        ) =>
                    {
                        crate::i18n::tr!("settings-conflict").into()
                    }
                    Err(error) => {
                        crate::i18n::tr!("settings-apply-error", error = format!("{:#}", error))
                    }
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
            Err(error) => {
                self.message =
                    crate::i18n::tr!("settings-reload-error", error = format!("{:#}", error))
            }
        }
    }

    fn control(&self, field: Field, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match field {
            Field::Language => gds_uim::el! {
                #div { id: $field.id(); test_support; } [
                    Select($&self.form.language) { cursor_pointer; }
                ]
            }
            .into_any_element(),
            Field::Source => gds_uim::el! {
                Select($&self.form.source) { cursor_pointer; }
            }
            .into_any_element(),
            Field::Mode => gds_uim::el! {
                Select($&self.form.mode) { cursor_pointer; }
            }
            .into_any_element(),
            Field::Background => gds_uim::el! {
                Select($&self.form.background) { cursor_pointer; }
            }
            .into_any_element(),
            Field::ComposerEnter => gds_uim::el! {
                #div { id: $field.id(); test_support; } [
                    Select($&self.form.composer_enter) { cursor_pointer; }
                ]
            }
            .into_any_element(),
            Field::UiFont => gds_uim::el! {
                Select($&self.form.ui_font) {
                    cursor_pointer;
                    search_placeholder: $crate::i18n::tr!("appearance-search-font");
                }
            }
            .into_any_element(),
            Field::MonoFont => gds_uim::el! {
                Select($&self.form.mono_font) {
                    cursor_pointer;
                    search_placeholder: $crate::i18n::tr!("appearance-search-font");
                }
            }
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
                let control = gds_uim::el! { Input($&input) {} };
                let reset = gds_uim::el! {
                    $super::action_button(format!("{}-default", field.id())) {
                        label: $crate::i18n::tr!("action-reset");
                        on_click: $cx.listener(move |this, _, window, cx| {
                            this.reset_input(&input, window, cx);
                        });
                    }
                };
                gds_uim::el! {
                    #div { flex; items_center; gap_2; } [
                        #div { id: $field.id(); test_support; flex_1; min_w_0; } [
                            $control
                        ],
                        $reset,
                    ]
                }
                .into_any_element()
            }
        }
    }
    fn row(&self, field: Field, last: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        gds_uim::el! {
            #div {
                id: $format!("{}-row", field.id()); test_support;
                flex; flex_shrink_0; items_center; justify_between; gap_3; py_3;
                @$(!last) => { border_b_1; border_color: $cx.theme().border; };
            } [
                #div { flex_1; min_w_0; flex; flex_col; gap_1; } [
                    $field.label(),
                    #div {
                        text_size: $crate::ui::SMALL;
                        text_color: $cx.theme().muted_foreground;
                    } [ $field.help() ],
                ],
                #div { w: $px(250.); flex_shrink_0; } [ $self.control(field, cx) ],
            ]
        }
        .into_any_element()
    }

    fn group_card(
        &self,
        group: Group,
        fields: &[Field],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let last = fields.len().saturating_sub(1);
        let title = gds_uim::el! {
            #div { flex; flex_col; gap_1; } [
                #div {
                    text_size: $crate::ui::HEADING; text_color: $cx.theme().foreground;
                } [ $group.title() ],
                #div {
                    text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [ $group.help() ],
            ]
        };
        gds_uim::el! {
            #div { id: $group.card_id(); test_support; } [
                GroupBox { outline; title: $title; } [
                    ..$fields.iter().enumerate().map(|(index, field)| self.row(*field, index == last, cx)),
                ],
            ]
        }.into_any_element()
    }

    fn reset_input(
        &mut self,
        input: &gpui_kit::Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.flush_pending(window, cx);
    }

    fn on_reset_all(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_form(&Preferences::default(), window, cx);
        self.save(window, cx);
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
        let header = gds_uim::el! {
            #div { p_5; flex_shrink_0; flex; flex_col; gap_2; } [
                #div { text_size: $crate::ui::TITLE; } [
                    $crate::i18n::tr!("settings-appearance")
                ],
                #div {
                    text_size: $crate::ui::SMALL;
                    text_color: $cx.theme().muted_foreground;
                } [
                    $crate::i18n::tr!(
                        "appearance-current-theme",
                        theme = (cx.global::<theme::Appearance>().name).to_string()
                    )
                ],
                #div {
                    text_size: $crate::ui::SMALL;
                    text_color: $cx.theme().muted_foreground;
                } [ $crate::i18n::tr!("appearance-autosave") ],
                ..$self.editor.theme_override().map(|path| gds_uim::el! {
                    #div {
                        text_size: $crate::ui::SMALL;
                        text_color: $cx.theme().muted_foreground;
                    } [
                        $crate::i18n::tr!("appearance-theme-override", path = (path.display()).to_string())
                    ]
                }),
            ]
        };
        let body = gds_uim::el! {
            #div {
                id: "appearance-sections"; flex_1; min_h_0; overflow_y_scroll;
                track_scroll: $&self.scroll; p_5; flex; flex_col; gap_5;
            } [
                ..$sections.is_empty().then(|| gds_uim::el! {
                    #div { id: "settings-no-results"; test_support; } [
                        $crate::i18n::tr!("settings-no-results")
                    ]
                }),
                ..$sections.iter().map(|(group, fields)| self.group_card(*group, fields, cx)),
            ]
        };
        let footer = gds_uim::el! {
            #div {
                p_3; flex_shrink_0; border_t_1; border_color: $cx.theme().border;
            } [
                #div { text_size: $crate::ui::SMALL; } [ $self.message.clone() ],
                #div { flex; items_center; justify_between; gap_3; } [
                    #div { min_w_0; text_size: $crate::ui::SMALL; truncate; } [
                        $crate::i18n::tr!("settings-file", path = (self.editor.appearance_path().display()).to_string())
                    ],
                    $super::action_button("settings-reset") {
                        label: $crate::i18n::tr!("appearance-reset-all");
                        disabled: $self.document.is_none();
                        on_click: $cx.listener(Self::on_reset_all);
                    },
                ],
            ]
        };
        gds_uim::el! {
            #div { size_full; flex; flex_col; } [ $header, $body, $footer ]
        }
    }
}
