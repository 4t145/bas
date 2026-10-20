use super::*;

#[test]
fn revision_is_a_base_not_a_clean_source_claim() {
    assert_eq!(
        revision_tag(Some("0123456789abcdef0123456789abcdef01234567")),
        "git-0123456789ab-worktree"
    );
    assert_eq!(
        revision_tag(Some(&"a".repeat(64))),
        "git-aaaaaaaaaaaa-worktree"
    );
}

#[test]
fn missing_or_invalid_revision_is_explicitly_unknown() {
    for value in [None, Some(""), Some("main"), Some("/private/repo")] {
        assert_eq!(revision_tag(value), "source-unknown");
    }
}

#[test]
fn explicit_tags_are_bounded_single_line_identifiers() {
    for value in ["release-0.1.0+ci.42", "nix-source-abc123", "dev_test"] {
        assert_eq!(validate_override(value), Ok(value));
    }
    for value in [
        "",
        "a\ncargo:rustc-env=OTHER=x",
        "/private/path",
        "two words",
        "版本",
    ] {
        assert!(validate_override(value).is_err());
    }
    assert!(validate_override(&"a".repeat(80)).is_ok());
    assert!(validate_override(&"a".repeat(81)).is_err());
}
