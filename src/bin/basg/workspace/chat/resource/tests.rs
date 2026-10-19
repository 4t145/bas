use super::*;

#[test]
fn remote_files_and_non_openable_schemes_are_not_local_launch_targets() {
    assert!(
        links::Context::default()
            .target("file:///tmp/sample.md")
            .is_err()
    );
    for uri in [
        "javascript:alert(1)",
        "data:text/html,<script>",
        "mcp://resource/one",
        "not a URI",
    ] {
        assert!(
            links::Context::new(None, true).target(uri).is_err(),
            "{uri}"
        );
    }
    assert!(
        links::Context::default()
            .target("https://example.com/resource")
            .is_ok()
    );
}
