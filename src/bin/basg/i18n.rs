//! Embedded Fluent catalogs. Static labels borrow cached text; messages with
//! arguments are formatted by Fluent, including its plural and bidi handling.

pub(crate) use bas::config::appearance::Language;
use fluent_bundle::{FluentArgs, FluentResource, concurrent::FluentBundle};
use std::{
    collections::HashMap,
    sync::{
        LazyLock,
        atomic::{AtomicU8, Ordering},
    },
};

const CHINESE: &str = include_str!("i18n/assets/zh-CN.ftl");
const ENGLISH: &str = include_str!("i18n/assets/en-US.ftl");
static LANGUAGE: AtomicU8 = AtomicU8::new(0);
static ZH: LazyLock<Catalog> = LazyLock::new(|| Catalog::new("zh-CN", CHINESE));
static EN: LazyLock<Catalog> = LazyLock::new(|| Catalog::new("en-US", ENGLISH));

struct Catalog {
    bundle: FluentBundle<FluentResource>,
    labels: HashMap<String, String>,
}

impl Catalog {
    fn new(locale: &str, source: &str) -> Self {
        let resource =
            FluentResource::try_new(source.to_owned()).expect("embedded Fluent catalog must parse");
        let locale: unic_langid::LanguageIdentifier = locale.parse().expect("valid locale");
        let mut bundle = FluentBundle::new_concurrent(vec![locale]);
        bundle
            .add_resource(resource)
            .expect("Fluent message IDs must be unique");
        let mut labels = HashMap::new();
        for line in source.lines() {
            if line.starts_with(char::is_alphabetic)
                && let Some((id, _)) = line.split_once(" =")
                && let Some(message) = bundle.get_message(id)
                && let Some(pattern) = message.value()
            {
                let mut errors = Vec::new();
                let value = bundle.format_pattern(pattern, None, &mut errors);
                if errors.is_empty() {
                    labels.insert(id.to_owned(), value.into_owned());
                }
            }
        }
        Self { bundle, labels }
    }

    fn format(&self, id: &str, args: &FluentArgs<'_>) -> Option<String> {
        let pattern = self.bundle.get_message(id)?.value()?;
        let mut errors = Vec::new();
        let value = self.bundle.format_pattern(pattern, Some(args), &mut errors);
        if !errors.is_empty() {
            tracing::error!(id, ?errors, "cannot format interface message");
            return None;
        }
        Some(value.into_owned())
    }
}

pub(crate) fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::English,
        _ => Language::Chinese,
    }
}

pub(crate) fn set_language(language: Language) {
    LANGUAGE.store(
        match language {
            Language::Chinese => 0,
            Language::English => 1,
        },
        Ordering::Relaxed,
    );
    // The published component library owns translations of its built-in editors
    // and selects. BasG's messages are supplied by the Fluent catalogs above.
    gpui_kit::component::set_locale(match language {
        Language::Chinese => "zh-CN",
        Language::English => "en",
    });
}

fn catalog(language: Language) -> &'static Catalog {
    match language {
        Language::Chinese => &ZH,
        Language::English => &EN,
    }
}

pub(crate) fn text(id: &'static str) -> &'static str {
    catalog(language())
        .labels
        .get(id)
        .or_else(|| ZH.labels.get(id))
        .map(String::as_str)
        .unwrap_or_else(|| {
            tracing::error!(id, "missing interface label");
            id
        })
}

pub(crate) fn format(id: &'static str, args: FluentArgs<'_>) -> String {
    catalog(language())
        .format(id, &args)
        .or_else(|| ZH.format(id, &args))
        .unwrap_or_else(|| id.to_owned())
}

macro_rules! tr {
    ($id:literal) => { $crate::i18n::text($id) };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = fluent_bundle::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::i18n::format($id, args)
    }};
}
pub(crate) use tr;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::visible_text;
