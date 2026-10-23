use super::*;

#[test]
fn appearance_search_uses_the_same_fields_and_groups_as_rendering() {
    let all = AppearancePage::matching_sections(&Query::new(" \t"));
    assert_eq!(all.len(), Group::ALL.len());
    assert_eq!(
        all.iter().map(|(_, fields)| fields.len()).sum::<usize>(),
        Field::ALL.len()
    );

    for query in ["Fonts", "字体", "Appearance Fonts"] {
        let sections = AppearancePage::matching_sections(&Query::new(query));
        assert_eq!(sections.len(), 1, "{query}");
        assert_eq!(sections[0].0, Group::Fonts, "{query}");
        assert_eq!(
            sections[0].1,
            vec![
                Field::UiFont,
                Field::ContentSize,
                Field::MonoFont,
                Field::Fallbacks
            ]
        );
    }
    assert_eq!(
        AppearancePage::matching_sections(&Query::new("界面字号")),
        vec![(Group::Fonts, vec![Field::UiFont])]
    );
    assert!(Field::UiFont.matches(&Query::new("Interface font size")));
    assert!(AppearancePage::matching_sections(&Query::new("no-such-setting")).is_empty());
    assert!(AppearancePage::matching_sections(&Query::new("Ctrl+Enter")).is_empty());
}

#[test]
fn session_routing_and_page_filtering_share_their_metadata() {
    for query in [
        "Enter",
        "Auto-collapse thoughts",
        "删除",
        "last agent",
        "对话偏好",
    ] {
        let search = Query::new(query);
        assert!(
            AppearancePage::search_sessions(&search).is_some(),
            "{query}"
        );
        assert!(AppearancePage::matches_session_query(query), "{query}");
    }
    assert!(AppearancePage::matches_session_query(" \t"));
    assert!(!AppearancePage::matches_session_query("no-such-setting"));
}
