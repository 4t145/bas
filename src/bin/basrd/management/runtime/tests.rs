use super::*;

#[test]
fn npm_launch_command_respects_declared_bin_and_rejects_ambiguous_or_escaping_bins() {
    assert_eq!(
        package_entrypoint(
            "@scope/agent",
            &serde_json::json!({"agent":"dist/serve.js","helper":"dist/helper.js"})
        )
        .expect("matching package bin"),
        PathBuf::from("dist/serve.js")
    );
    assert!(
        package_entrypoint("agent", &serde_json::json!({"one":"one.js","two":"two.js"})).is_err()
    );
    assert!(package_entrypoint("agent", &serde_json::json!("../../escape.js")).is_err());
}
