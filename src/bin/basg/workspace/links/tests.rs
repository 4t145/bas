use super::*;

#[test]
fn remote_paths_and_unsupported_schemes_never_launch_local_resources() {
    let remote = Context::default();
    for address in ["file:///tmp/notes.txt", "/tmp/notes.txt", "notes.txt"] {
        assert!(remote.target(address).is_err(), "{address}");
    }
    for address in [
        "",
        "#section",
        "?query",
        "javascript:alert(1)",
        "data:text/html,hello",
        "mcp://resource",
    ] {
        assert!(
            Context::new(None, true).target(address).is_err(),
            "{address}"
        );
    }
    for address in [
        "https://example.com/file#part",
        "http://example.com",
        "mailto:test@example.com",
    ] {
        assert!(matches!(remote.target(address), Ok(Target::External(_))));
    }
}
