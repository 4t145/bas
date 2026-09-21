use super::AppearancePage;
use crate::theme::{
    self,
    preferences::{Background, Preferences, ThemeMode, ThemeSource},
};
use gpui_kit::{
    App, AppContext, Context, Entity, Window,
    component::{IndexPath, input::*, select::*},
};

pub(super) struct Form {
    pub ui_font: Entity<SelectState<SearchableVec<FontChoice>>>,
    pub mono_font: Entity<SelectState<SearchableVec<FontChoice>>>,
    pub fallbacks: Entity<InputState>,
    pub ui_size: Entity<InputState>,
    pub content_size: Entity<InputState>,
    pub line_height: Entity<InputState>,
    pub opacity: Entity<InputState>,
    pub theme_file: Entity<InputState>,
    pub source: Entity<SelectState<Vec<ThemeSource>>>,
    pub mode: Entity<SelectState<Vec<ThemeMode>>>,
    pub background: Entity<SelectState<Vec<Background>>>,
}

impl SelectItem for ThemeSource {
    type Value = Self;
    fn title(&self) -> gpui_kit::SharedString {
        match self {
            Self::Desktop => "Default · 桌面",
            Self::Builtin => "内置",
        }
        .into()
    }
    fn value(&self) -> &Self {
        self
    }
}

impl SelectItem for ThemeMode {
    type Value = Self;
    fn title(&self) -> gpui_kit::SharedString {
        match self {
            Self::Desktop => "Default · 桌面",
            Self::System => "系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
        .into()
    }
    fn value(&self) -> &Self {
        self
    }
}

impl SelectItem for Background {
    type Value = Self;
    fn title(&self) -> gpui_kit::SharedString {
        match self {
            Self::Auto => "自动",
            Self::Opaque => "不透明",
            Self::Transparent => "透明",
            Self::Blurred => "Default · 模糊",
        }
        .into()
    }
    fn value(&self) -> &Self {
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum DraftError {
    #[error("{0}：请输入数字或留空继承")]
    Number(&'static str),
    #[error("配置值无效：{0}")]
    Invalid(#[from] theme::preferences::ValidationError),
}

impl Form {
    pub fn new(p: &Preferences, window: &mut Window, cx: &mut Context<AppearancePage>) -> Self {
        let mut input = |value: String| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("留空继承");
                input.set_value(value, window, cx);
                input
            })
        };
        Self {
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
            ui_font_size: number(&self.ui_size, "UI 字号")?,
            content_font_size: number(&self.content_size, "正文字号")?,
            content_line_height: number(&self.line_height, "正文行高")?,
            content_opacity: number(&self.opacity, "内容不透明度")?,
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
        };
        p.validate()?;
        Ok(p)
    }
}

fn choice<T: SelectItem<Value = T> + PartialEq + 'static>(
    items: Vec<T>,
    selected: T,
    window: &mut Window,
    cx: &mut Context<AppearancePage>,
) -> Entity<SelectState<Vec<T>>> {
    let index = items
        .iter()
        .position(|item| *item == selected)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(items, index, window, cx))
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
            Self::Default => "Default · 继承".into(),
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
