//! Settings search uses the same localized titles and help as the rendered pages.

use crate::i18n::{self, Language};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct Section {
    title: &'static str,
    help: Option<&'static str>,
}

impl Section {
    pub(super) const fn new(title: &'static str, help: &'static str) -> Self {
        Self {
            title,
            help: Some(help),
        }
    }

    pub(super) const fn heading(title: &'static str) -> Self {
        Self { title, help: None }
    }

    pub(super) fn title(self) -> &'static str {
        i18n::text(self.title)
    }

    pub(super) fn help(self) -> &'static str {
        self.help.map(i18n::text).unwrap_or_default()
    }

    pub(super) fn score(self, query: &Query) -> Option<Match> {
        self.score_under(query, &[])
    }

    pub(super) fn score_under(self, query: &Query, parents: &[Self]) -> Option<Match> {
        // Both shipped languages remain searchable without changing the UI locale.
        [Language::Chinese, Language::English]
            .into_iter()
            .filter_map(|language| {
                let title = i18n::text_in(language, self.title);
                let titles = parents
                    .iter()
                    .map(|parent| i18n::text_in(language, parent.title))
                    .chain(std::iter::once(title))
                    .collect::<Vec<_>>()
                    .join(" ");
                let help = parents
                    .iter()
                    .chain(std::iter::once(&self))
                    .filter_map(|section| section.help)
                    .map(|id| i18n::text_in(language, id))
                    .collect::<Vec<_>>()
                    .join(" ");
                query.score(title, &help).max(query.score(&titles, &help))
            })
            .max()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Match {
    Help,
    Title,
    ExactTitle,
}

pub(super) struct Query(String);

impl Query {
    pub(super) fn new(value: &str) -> Self {
        Self(
            value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
        )
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, text: &str) -> bool {
        self.0.split_whitespace().all(|word| text.contains(word))
    }

    pub(super) fn score(&self, title: &str, help: &str) -> Option<Match> {
        if self.is_empty() {
            return None;
        }
        let title = title.to_lowercase();
        if title == self.0 {
            Some(Match::ExactTitle)
        } else if self.contains(&title) {
            Some(Match::Title)
        } else if self.contains(&format!("{title} {}", help.to_lowercase())) {
            Some(Match::Help)
        } else {
            None
        }
    }
}
