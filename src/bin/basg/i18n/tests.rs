use super::*;
pub(crate) fn visible_text(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}
use std::collections::{BTreeMap, BTreeSet};

fn messages(source: &str) -> BTreeMap<&str, String> {
    let mut messages = BTreeMap::new();
    let mut current = None;
    for line in source.lines() {
        if line.starts_with(char::is_alphabetic) {
            let (id, value) = line.split_once(" =").expect("message assignment");
            messages.insert(id, value.to_owned());
            current = Some(id);
        } else if line.starts_with(' ')
            && let Some(id) = current
        {
            messages
                .get_mut(id)
                .expect("current message")
                .push_str(line);
        }
    }
    messages
}

fn variables(pattern: &str) -> BTreeSet<String> {
    regex::Regex::new(r"\$([a-zA-Z][a-zA-Z0-9_-]*)")
        .expect("variable regex")
        .captures_iter(pattern)
        .map(|capture| capture[1].to_owned())
        .collect()
}

#[test]
fn catalogs_parse_and_have_matching_messages_and_arguments() {
    let zh = Catalog::new("zh-CN", CHINESE);
    let en = Catalog::new("en-US", ENGLISH);
    let chinese = messages(CHINESE);
    let english = messages(ENGLISH);
    assert_eq!(
        chinese.keys().collect::<Vec<_>>(),
        english.keys().collect::<Vec<_>>()
    );
    for (id, pattern) in &chinese {
        let names = variables(pattern);
        assert_eq!(names, variables(&english[id]), "argument mismatch: {id}");
        for count in [0, 1, 2, 100] {
            let mut args = FluentArgs::new();
            for name in &names {
                args.set(name.as_str(), count);
            }
            for catalog in [&zh, &en] {
                assert!(catalog.format(id, &args).is_some(), "invalid message: {id}");
                if names.is_empty() {
                    assert!(catalog.labels.contains_key(*id), "uncached label: {id}");
                }
            }
        }
    }
}

#[test]
fn counts_use_english_plural_rules() {
    let en = Catalog::new("en-US", ENGLISH);
    for (count, expected) in [(0, "0 tool calls"), (1, "1 tool call"), (2, "2 tool calls")] {
        let mut args = FluentArgs::new();
        args.set("tools", count);
        let value = en
            .format("tool-count", &args)
            .expect("valid plural message");
        assert_eq!(value.replace(['\u{2068}', '\u{2069}'], ""), expected);
    }
}

#[test]
fn values_are_data_and_keep_bidi_isolation() {
    let mut args = FluentArgs::new();
    let path = "/tmp/中文/{ $secret }/مرحبا.txt";
    args.set("path", path);
    let result = EN
        .format("settings-file", &args)
        .expect("valid path message");
    assert!(result.contains(path));
    assert!(result.contains('\u{2068}'));
    assert!(result.contains('\u{2069}'));
}
