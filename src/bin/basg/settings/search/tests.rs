use super::*;
use crate::settings::{Category, Scope};

#[test]
fn settings_search_normalizes_words_and_ranks_display_text() {
    let query = Query::new("  FONT\tSIZE \n");
    assert_eq!(query.score("Font size", ""), Some(Match::ExactTitle));
    assert_eq!(query.score("Content font size", ""), Some(Match::Title));
    assert_eq!(query.score("Font", "Size in pixels"), Some(Match::Help));
    assert_eq!(query.score("Font", "Choose a family"), None);
    assert_eq!(Query::new("").score("Font size", ""), None);
}

#[test]
fn every_category_is_searchable_by_its_displayed_title_in_both_languages() {
    for category in Scope::ALL.into_iter().flat_map(Scope::categories) {
        for language in [Language::Chinese, Language::English] {
            let title = i18n::text_in(language, category.section().title);
            assert_eq!(
                Category::for_query(&Query::new(title)),
                Some(*category),
                "{title}"
            );
        }
    }
}

#[test]
fn settings_search_routes_real_bilingual_fields_and_help() {
    for (category, queries) in [
        (
            Category::Appearance,
            &[
                "Interface language",
                "界面语言",
                "Appearance font size",
                "外观 字体",
                "Code font",
                "Background opacity",
                "像素",
            ][..],
        ),
        (
            Category::Sessions,
            &[
                "Enter",
                "Ctrl+Enter",
                "Auto-collapse thoughts",
                "删除",
                "Default agent for new conversations",
            ][..],
        ),
        (
            Category::Connections,
            &[
                "Server address",
                "服务器",
                "端点",
                "Connection name",
                "Client certificate",
                "mTLS",
            ][..],
        ),
        (
            Category::Agents,
            &[
                "Arguments",
                "参数",
                "命令",
                "Command",
                "ACP Arguments",
                "Agent ID",
            ][..],
        ),
        (
            Category::SessionManagement,
            &[
                " ACP ",
                "POOL",
                "池",
                "idle",
                "预热",
                "回收",
                "Reserve size",
                "ACP pool",
            ][..],
        ),
    ] {
        for query in queries {
            assert_eq!(
                Category::for_query(&Query::new(query)),
                Some(category),
                "{query}"
            );
        }
    }
}

#[test]
fn exact_titles_beat_incidental_help_and_ties_have_stable_ownership() {
    // ACP occurs in both backend pages; reserve/idle policy owns an equal match.
    assert_eq!(
        Category::for_query(&Query::new("ACP")),
        Some(Category::SessionManagement)
    );
    // "Enter" also appears in appearance help, but the conversation field is a title.
    assert_eq!(
        Category::for_query(&Query::new("Enter")),
        Some(Category::Sessions)
    );
    // The session page mentions agents, but a category's complete title wins.
    assert_eq!(
        Category::for_query(&Query::new("Agents")),
        Some(Category::Agents)
    );
    assert_eq!(
        Category::for_query(&Query::new("Connections")),
        Some(Category::Connections)
    );
}

#[test]
fn empty_unknown_and_matched_queries_have_explicit_navigation_defaults() {
    assert_eq!(Category::default(), Category::Appearance);
    for current in Scope::ALL.into_iter().flat_map(Scope::categories).copied() {
        for blank in ["", " \n\t "] {
            let query = Query::new(blank);
            assert_eq!(Category::for_query(&query), None);
            assert_eq!(current.searched(&query, None), current);
        }
        let query = Query::new("no-such-setting-123");
        assert_eq!(Category::for_query(&query), None);
        assert_eq!(
            current.searched(&query, None),
            if current == Category::Agents {
                Category::Agents
            } else {
                Category::Appearance
            }
        );
        assert_eq!(
            current.searched(&Query::new("Fonts"), None),
            Category::Appearance
        );
    }
}

#[test]
fn actual_agent_metadata_competes_by_relevance_instead_of_brand_whitelists() {
    let query = Query::new("my-private-agent");
    assert_eq!(Category::for_query(&query), None);
    assert_eq!(
        Category::Appearance.searched(&query, query.score("my-private-agent", "")),
        Category::Agents,
    );
    assert_eq!(
        Category::Agents.searched(&Query::new("Fonts"), Some(Match::Help)),
        Category::Appearance,
    );
    assert_eq!(
        Category::Appearance.searched(&Query::new("ACP"), Some(Match::Help)),
        Category::SessionManagement,
    );
}
