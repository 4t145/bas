use super::*;

fn manifest(distribution: serde_json::Value) -> Manifest {
    serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","version":"1.2.3","distribution":distribution})).expect("valid fixture")
}

#[test]
fn binary_dispatch_precedes_pinned_npm_and_unsupported_uvx_stays_visible() {
    let entry = manifest(
        serde_json::json!({"binary":{platform():{"archive":"https://example.com/agent.tgz","cmd":"./agent","args":["argument with spaces"],"env":{"TOKEN":"literal $value"}}},"npx":{"package":"fixture@1.2.3"}}),
    );
    let Selected::Binary(binary) = entry.select().expect("supported binary") else {
        panic!("binary should take precedence")
    };
    assert_eq!(binary.args, ["argument with spaces"]);
    assert_eq!(binary.env["TOKEN"], "literal $value");
    assert!(matches!(
        manifest(serde_json::json!({"uvx":{"package":"fixture"}})).select(),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn unsafe_paths_urls_versions_and_schema_are_rejected() {
    for path in [
        "../agent",
        "/bin/agent",
        "C:\\agent.exe",
        "a/../../b",
        "a\\..\\b",
        "./",
    ] {
        assert!(safe_relative(path).is_err(), "{path}");
    }
    assert_eq!(
        safe_relative(".\\dist\\agent.exe").expect("relative Windows command"),
        PathBuf::from("dist/agent.exe")
    );
    for url in [
        "file:///tmp/agent",
        "http://example.com/agent",
        "https://user:password@example.com/agent",
        "https://example.com/agent#fragment",
    ] {
        assert!(download_url(url).is_err());
    }
    assert!(download_url("http://127.0.0.1:1234/agent").is_ok());
    assert!(download_url("http://[::1]:1234/agent").is_ok());
    for package in [
        "fixture@latest",
        "fixture@^1.2.3",
        "https://evil/fixture@1.2.3",
        "--prefix@1.2.3",
        "fixture@1.2.4",
    ] {
        assert!(pinned_package(package, "1.2.3").is_err(), "{package}");
    }
    assert_eq!(
        pinned_package("@scope/fixture@1.2.3", "1.2.3").expect("pinned scoped package"),
        "@scope/fixture"
    );
    let mut registry = Registry {
        version: "1.0.0".into(),
        agents: vec![manifest(
            serde_json::json!({"npx":{"package":"fixture@1.2.3"}}),
        )],
    };
    assert!(registry.validate().is_ok());
    registry.agents.push(registry.agents[0].clone());
    assert!(registry.validate().is_err());
    registry.agents.pop();
    registry.version = "2.0.0".into();
    assert!(registry.validate().is_err());
}
