//! Exercise the actual Manager → runtime → process ownership chain without npm or downloads.

use super::*;
use crate::management::{Manager, registry};
use std::{fs, path::PathBuf, sync::Arc};

const CONTROLLER: &str = "management::runtime::process::tests::installation::fixture";
const CASE: &str = "BAS_INSTALL_PROCESS_TEST_CASE";
const EXECUTABLE: &str = "BAS_INSTALL_PROCESS_TEST_EXECUTABLE";

#[tokio::test]
async fn fixture() {
    let Ok(case) = std::env::var(CASE) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("BAS_ACP_DATA_DIR").expect("isolated data root"));
    let store = bas::config::Store::at(root.join("settings.json"));
    let manager = Arc::new(Manager::new(Some(store.clone())).expect("manager"));
    let root_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("root listener");
    let leaf_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("leaf listener");
    let manifest = serde_json::from_value(serde_json::json!({
        "id": "fixture-agent", "name": "Fixture", "version": "1.0.0",
        "distribution": {"npx": {
            "package": "fixture-agent@1.0.0",
            "env": {
                MODE: "root",
                ROOT_ADDRESS: address(&root_listener).to_string(),
                LEAF_ADDRESS: address(&leaf_listener).to_string()
            }
        }}
    }))
    .expect("npm fixture manifest");
    manager.registry.lock().await.registry = Some(registry::Registry {
        version: "1.0.0".into(),
        agents: vec![manifest],
    });
    let (send, receive) = tokio::sync::mpsc::channel(16);
    let owner = Arc::clone(&manager);
    let waiter = tokio::spawn(async move { owner.install("fixture-agent", send).await });
    let mut abort_on_failure = AbortOnDrop(Some(waiter.abort_handle()));
    let mut child = Peer::accept(&root_listener).await;
    let mut leaf = Peer::accept(&leaf_listener).await;
    let versions = root.join("agents/fixture-agent");
    assert!(fs::read_dir(&versions).expect("staging").next().is_some());
    assert!(manager.mutation.try_lock().is_err());
    assert!(manager.mutation_lock().is_err());
    match case.as_str() {
        "drop" => {
            waiter.abort();
            assert!(
                bounded(waiter)
                    .await
                    .expect_err("aborted caller")
                    .is_cancelled()
            );
        }
        "progress" => {
            drop(receive);
            assert!(bounded(waiter).await.expect("caller task").is_err());
        }
        "shutdown" => {
            bounded(manager.shutdown()).await;
            assert!(bounded(waiter).await.expect("caller task").is_err());
        }
        _ => panic!("unknown fixture case"),
    }
    abort_on_failure.0.take();
    // For dropped waiters, this observes the operation's own cancellation;
    // it does not introduce an independent shutdown cancellation.
    manager.installations.close();
    bounded(manager.installations.wait()).await;
    assert_reaped(child.pid);
    child.assert_closed().await;
    leaf.assert_closed().await;
    assert!(
        fs::read_dir(versions)
            .expect("clean staging")
            .next()
            .is_none()
    );
    assert!(!store.path().exists(), "cancelled install cannot activate");
    assert!(manager.mutation.try_lock().is_ok());
    assert!(manager.mutation_lock().is_ok());
    assert_eq!(Arc::strong_count(&manager), 1);
    manager.shutdown().await;
}

#[tokio::test]
async fn cancelled_npm_operations_reap_before_releasing_staging_and_locks() {
    use std::os::unix::fs::PermissionsExt;
    for case in ["drop", "progress", "shutdown"] {
        let directory = tempfile::tempdir().expect("isolated process environment");
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).expect("fixture bin");
        let node = bin.join("node");
        fs::write(
            &node,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'v22.0.0\\n'; exit 0; fi\nexec \"${EXECUTABLE}\" --exact {FIXTURE} --nocapture\n"
            ),
        )
        .expect("node fixture");
        fs::write(bin.join("npm"), "#!/bin/sh\nexit 1\n").expect("npm entrypoint fixture");
        for path in [&node, &bin.join("npm")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("executable");
        }
        let mut command =
            tokio::process::Command::new(std::env::current_exe().expect("test executable"));
        command
            .args(["--exact", CONTROLLER, "--nocapture"])
            .env(CASE, case)
            .env(
                EXECUTABLE,
                std::env::current_exe().expect("test executable"),
            )
            .env("PATH", &bin)
            .env("BAS_ACP_DATA_DIR", directory.path().join("data"))
            .env("BAS_ACP_REGISTRY_URL", "http://127.0.0.1:1/unused")
            .kill_on_drop(true);
        let output = bounded(command.output()).await.expect("controller");
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
