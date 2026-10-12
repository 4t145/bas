use super::*;
pub(crate) fn visible_text(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

// Both BASR and the upstream components keep the locale at process scope.
// Run tests that change it in a child process so other GUI tests stay isolated.
#[cfg(feature = "gui-tests")]
pub(crate) fn isolated_language_test(name: &str) -> bool {
    const CHILD: &str = "BASRW_LANGUAGE_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name)
        .output()
        .expect("run isolated language test");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

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

#[test]
fn every_interface_message_reference_exists() {
    fn check(path: &Path, ids: &BTreeMap<&str, String>, pattern: &regex::Regex) {
        for entry in std::fs::read_dir(path).expect("source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                check(&path, ids, pattern);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).expect("source file");
                for capture in pattern.captures_iter(&source) {
                    assert!(
                        ids.contains_key(&capture[1]),
                        "missing {} in {}",
                        &capture[1],
                        path.display()
                    );
                }
            }
        }
    }
    let pattern =
        regex::Regex::new(r#"tr!\(\s*"([a-z][a-z0-9-]*)""#).expect("message reference regex");
    check(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/basrw"),
        &messages(CHINESE),
        &pattern,
    );
}

#[test]
fn language_preferences_are_strict_and_backwards_compatible() {
    use basr::config::appearance::Appearance;
    assert_eq!(
        Appearance::from_json("{}")
            .expect("default preferences")
            .language,
        Language::Chinese
    );
    for (locale, language) in [("zh-CN", Language::Chinese), ("en-US", Language::English)] {
        let preferences = Appearance::from_json(&format!(r#"{{"language":"{locale}"}}"#))
            .expect("supported language");
        assert_eq!(preferences.language, language);
        let serialized = serde_json::to_value(preferences).expect("serialized preferences");
        assert_eq!(serialized["language"], locale);
    }
    assert!(Appearance::from_json(r#"{"language":"fr"}"#).is_err());
}
