use super::AppearancePage;
use crate::theme::{
    self,
    preferences::{
        Background, ComposerEnterBehavior, Language, Preferences, ThemeMode, ThemeSource,
    },
};
use gpui_kit::{
    App, AppContext, Context, Entity, Window,
    component::{IndexPath, input::*, select::*},
};

pub(super) struct Form {
    pub confirm_delete: Option<bool>,
    pub auto_collapse_activity: Option<bool>,
    pub default_agent: Entity<SelectState<Vec<Choice<Option<String>>>>>,
    pub default_agent_choices: Vec<Option<String>>,
    pub language: Entity<SelectState<Vec<Choice<Language>>>>,
    pub ui_font: Entity<SelectState<SearchableVec<FontChoice>>>,
    pub mono_font: Entity<SelectState<SearchableVec<FontChoice>>>,
    pub fallbacks: Entity<InputState>,
    pub ui_size: Entity<InputState>,
    pub content_size: Entity<InputState>,
    pub line_height: Entity<InputState>,
    pub opacity: Entity<InputState>,
    pub theme_file: Entity<InputState>,
    pub source: Entity<SelectState<Vec<Choice<ThemeSource>>>>,
    pub mode: Entity<SelectState<Vec<Choice<ThemeMode>>>>,
    pub background: Entity<SelectState<Vec<Choice<Background>>>>,
    pub composer_enter: Entity<SelectState<Vec<Choice<ComposerEnterBehavior>>>>,
}

#[derive(Clone)]
pub(super) struct Choice<T>(pub T);

impl SelectItem for Choice<Option<String>> {
    type Value = Option<String>;
    fn title(&self) -> gpui_kit::SharedString {
        self.0
            .clone()
            .unwrap_or_else(|| crate::i18n::tr!("settings-last-agent").into())
            .into()
    }
    fn value(&self) -> &Self::Value {
        &self.0
    }
}

impl SelectItem for Choice<Language> {
    type Value = Language;
    fn title(&self) -> gpui_kit::SharedString {
        match self.0 {
            Language::Chinese => "简体中文",
            Language::English => "English",
        }
        .into()
    }
    fn value(&self) -> &Language {
        &self.0
    }
}

impl SelectItem for Choice<ThemeSource> {
    type Value = ThemeSource;
    fn title(&self) -> gpui_kit::SharedString {
        match self.0 {
            ThemeSource::Desktop => crate::i18n::tr!("appearance-desktop"),
            ThemeSource::Builtin => crate::i18n::tr!("appearance-builtin"),
        }
        .into()
    }
    fn value(&self) -> &ThemeSource {
        &self.0
    }
}

impl SelectItem for Choice<ThemeMode> {
    type Value = ThemeMode;
    fn title(&self) -> gpui_kit::SharedString {
        match self.0 {
            ThemeMode::Desktop => crate::i18n::tr!("appearance-desktop"),
            ThemeMode::System => crate::i18n::tr!("appearance-system"),
            ThemeMode::Light => crate::i18n::tr!("appearance-light"),
            ThemeMode::Dark => crate::i18n::tr!("appearance-dark"),
        }
        .into()
    }
    fn value(&self) -> &ThemeMode {
        &self.0
    }
}

impl SelectItem for Choice<Background> {
    type Value = Background;
    fn title(&self) -> gpui_kit::SharedString {
        match self.0 {
            Background::Auto => crate::i18n::tr!("appearance-auto"),
            Background::Opaque => crate::i18n::tr!("appearance-opaque"),
            Background::Transparent => crate::i18n::tr!("appearance-transparent"),
            Background::Blurred => crate::i18n::tr!("appearance-blurred"),
        }
        .into()
    }
    fn value(&self) -> &Background {
        &self.0
    }
}

impl SelectItem for Choice<ComposerEnterBehavior> {
    type Value = ComposerEnterBehavior;
    fn title(&self) -> gpui_kit::SharedString {
        match self.0 {
            ComposerEnterBehavior::Send => crate::i18n::tr!("composer-enter-send"),
            ComposerEnterBehavior::Newline => crate::i18n::tr!("composer-enter-newline"),
        }
        .into()
    }
    fn value(&self) -> &ComposerEnterBehavior {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum DraftError {
    #[error("{}", crate::i18n::tr!("appearance-invalid-number", field = .0.to_string()))]
    Number(&'static str),
    #[error("{}", validation_message(.0))]
    Invalid(#[from] theme::preferences::ValidationError),
}

fn validation_message(error: &theme::preferences::ValidationError) -> String {
    use theme::preferences::ValidationError;
    let label = |field| match field {
        "ui_font_size" => crate::i18n::tr!("appearance-ui-size"),
        "content_font_size" => crate::i18n::tr!("appearance-content-size"),
        "content_line_height" => crate::i18n::tr!("appearance-line-height"),
        "ui_font_family" => crate::i18n::tr!("appearance-ui-font"),
        "mono_font_family" => crate::i18n::tr!("appearance-mono-font"),
        "font_fallbacks" => crate::i18n::tr!("appearance-fallbacks"),
        _ => crate::i18n::tr!("settings-appearance"),
    };
    match error {
        ValidationError::Positive(field) => {
            crate::i18n::tr!("appearance-positive", field = label(field))
        }
        ValidationError::Opacity => crate::i18n::tr!("appearance-opacity-range").into(),
        ValidationError::Font(field) => {
            crate::i18n::tr!("appearance-font-required", field = label(field))
        }
        ValidationError::ThemeFile => crate::i18n::tr!("appearance-path-required").into(),
    }
}

impl Form {
    pub fn new(p: &Preferences, window: &mut Window, cx: &mut Context<AppearancePage>) -> Self {
        let default_agent_choices: Vec<_> = std::iter::once(None)
            .chain(p.default_agent.clone().map(Some))
            .collect();
        let default_agent = choice(
            default_agent_choices.clone(),
            p.default_agent.clone(),
            window,
            cx,
        );
        let mut input = |value: String| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(crate::i18n::tr!("appearance-inherit-placeholder"));
                input.set_value(value, window, cx);
                input
            })
        };
        Self {
            confirm_delete: p.confirm_session_delete,
            auto_collapse_activity: p.auto_collapse_activity,
            default_agent,
            default_agent_choices,
            fallbacks: input(
                p.font_fallbacks
                    .as_ref()
                    .map(|fonts| {
                        if fonts.is_empty() {
                            "[]".into()
                        } else {
                            fonts.join(", ")
                        }
                    })
                    .unwrap_or_default(),
            ),
            ui_size: input(p.ui_font_size.map(|v| v.to_string()).unwrap_or_default()),
            content_size: input(
                p.content_font_size
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            line_height: input(
                p.content_line_height
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            opacity: input(p.content_opacity.map(|v| v.to_string()).unwrap_or_default()),
            theme_file: input(
                p.theme_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            language: choice(
                vec![Language::Chinese, Language::English],
                p.language,
                window,
                cx,
            ),
            ui_font: font_choice(p.ui_font_family.clone(), window, cx),
            mono_font: font_choice(p.mono_font_family.clone(), window, cx),
            source: choice(
                vec![ThemeSource::Desktop, ThemeSource::Builtin],
                p.theme_source,
                window,
                cx,
            ),
            mode: choice(
                vec![
                    ThemeMode::Desktop,
                    ThemeMode::System,
                    ThemeMode::Light,
                    ThemeMode::Dark,
                ],
                p.theme_mode,
                window,
                cx,
            ),
            background: choice(
                vec![
                    Background::Auto,
                    Background::Opaque,
                    Background::Transparent,
                    Background::Blurred,
                ],
                p.background,
                window,
                cx,
            ),
            composer_enter: choice(
                vec![ComposerEnterBehavior::Send, ComposerEnterBehavior::Newline],
                p.composer_enter_behavior,
                window,
                cx,
            ),
        }
    }

    /// # Errors
    /// Rejects malformed numeric input and invalid preference values.
    pub fn candidate(&self, cx: &App) -> Result<Preferences, DraftError> {
        let optional = |input: &Entity<InputState>| {
            let value = input.read(cx).value().trim().to_owned();
            (!value.is_empty()).then_some(value)
        };
        let number = |input: &Entity<InputState>, label| {
            optional(input)
                .map(|v| v.parse().map_err(|_| DraftError::Number(label)))
                .transpose()
        };
        let p = Preferences {
            confirm_session_delete: self.confirm_delete,
            auto_collapse_activity: self.auto_collapse_activity,
            default_agent: self
                .default_agent
                .read(cx)
                .selected_value()
                .cloned()
                .flatten(),
            language: self
                .language
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            ui_font_family: self
                .ui_font
                .read(cx)
                .selected_value()
                .and_then(FontChoice::name),
            mono_font_family: self
                .mono_font
                .read(cx)
                .selected_value()
                .and_then(FontChoice::name),
            font_fallbacks: optional(&self.fallbacks).map(|v| {
                if v == "[]" {
                    vec![]
                } else {
                    v.split(',').map(|v| v.trim().to_owned()).collect()
                }
            }),
            ui_font_size: number(&self.ui_size, crate::i18n::tr!("appearance-ui-size"))?,
            content_font_size: number(
                &self.content_size,
                crate::i18n::tr!("appearance-content-size"),
            )?,
            content_line_height: number(
                &self.line_height,
                crate::i18n::tr!("appearance-line-height"),
            )?,
            content_opacity: number(&self.opacity, crate::i18n::tr!("appearance-opacity"))?,
            theme_file: optional(&self.theme_file).map(Into::into),
            theme_source: self
                .source
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            theme_mode: self
                .mode
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            background: self
                .background
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            composer_enter_behavior: self
                .composer_enter
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
        };
        p.validate()?;
        Ok(p)
    }
}

fn choice<T: Clone + PartialEq + 'static>(
    items: Vec<T>,
    selected: T,
    window: &mut Window,
    cx: &mut Context<AppearancePage>,
) -> Entity<SelectState<Vec<Choice<T>>>>
where
    Choice<T>: SelectItem<Value = T>,
{
    let index = items
        .iter()
        .position(|item| *item == selected)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(items.into_iter().map(Choice).collect(), index, window, cx))
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FontChoice {
    Default,
    Named(String),
}

impl FontChoice {
    fn name(&self) -> Option<String> {
        match self {
            Self::Default => None,
            Self::Named(name) => Some(name.clone()),
        }
    }
}

impl SelectItem for FontChoice {
    type Value = Self;
    fn title(&self) -> gpui_kit::SharedString {
        match self {
            Self::Default => crate::i18n::tr!("appearance-font-default").into(),
            Self::Named(name) => name.clone().into(),
        }
    }
    fn value(&self) -> &Self {
        self
    }
}

fn font_choice(
    selected: Option<String>,
    window: &mut Window,
    cx: &mut Context<AppearancePage>,
) -> Entity<SelectState<SearchableVec<FontChoice>>> {
    let mut names = cx.text_system().all_font_names();
    if let Some(name) = &selected {
        names.push(name.clone());
    }
    names.sort();
    names.dedup();
    let items = std::iter::once(FontChoice::Default)
        .chain(names.into_iter().map(FontChoice::Named))
        .collect::<Vec<_>>();
    let selected = selected
        .map(FontChoice::Named)
        .unwrap_or(FontChoice::Default);
    let index = items
        .iter()
        .position(|item| *item == selected)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(SearchableVec::new(items), index, window, cx).searchable(true))
}
