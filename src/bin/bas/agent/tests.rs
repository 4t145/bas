use bas::acp;
use std::sync::Arc;

use super::{
    AgentSpec, Catalog, CatalogError, Command, Host, LocalHost, PermissionKind, PermissionOutcome,
    PermissionPolicy, PermissionRequest,
};
use bas::model::SessionId;

#[test]
fn catalog_rejects_duplicate_ids() {
    let mut catalog = Catalog::with_defaults();
    let error = catalog
        .insert(
            AgentSpec::embedded(),
            Arc::new(super::embedded::Driver::new()),
        )
        .unwrap_err();

    assert!(matches!(error, CatalogError::Duplicate(_)));
}

#[test]
fn catalog_requires_a_command_for_external_agents() {
    let mut catalog = Catalog::new();
    let mut spec = AgentSpec::acp("opencode", Command::new("opencode", Vec::new()));
    spec.command = None;

    let error = catalog
        .insert(spec, Arc::new(super::acp::Driver::new()))
        .unwrap_err();

    assert!(matches!(error, CatalogError::MissingCommand { .. }));
}

#[tokio::test]
async fn catalog_reports_unknown_backends() {
    let catalog = Catalog::with_defaults();
    let error = catalog
        .launch(super::LaunchRequest {
            session: SessionId::new(),
            spec: Arc::new(AgentSpec::acp("missing", Command::new("nope", Vec::new()))),
            cwd: std::env::current_dir().expect("working directory exists"),
            additional_directories: Vec::new(),
            host: Arc::new(LocalHost::default()),
            updates: tokio::sync::mpsc::channel(16).0,
        })
        .await
        .err()
        .expect("the backend is not configured");

    assert!(error.to_string().contains("not configured"));
}

#[tokio::test]
async fn local_host_follows_its_permission_policy() {
    let request = PermissionRequest::new(
        "test",
        acp::ToolCallUpdate::new(
            "call-1",
            acp::ToolCallUpdateFields::new().title("write a file"),
        ),
        vec![
            acp::PermissionOption::new("allow", "Allow once", PermissionKind::AllowOnce),
            acp::PermissionOption::new("reject", "Reject", PermissionKind::RejectOnce),
        ],
    );
    let allowed = LocalHost::new(PermissionPolicy::Allow)
        .request_permission(request.clone())
        .await
        .unwrap();
    let denied = LocalHost::new(PermissionPolicy::Deny)
        .request_permission(request)
        .await
        .unwrap();

    assert_eq!(
        allowed,
        PermissionOutcome::Selected(acp::SelectedPermissionOutcome::new("allow"))
    );
    assert_eq!(
        denied,
        PermissionOutcome::Selected(acp::SelectedPermissionOutcome::new("reject"))
    );
}

#[tokio::test]
async fn supervised_panics_become_backend_failures() {
    let (updates, mut receiver) = tokio::sync::mpsc::channel(1);
    let task = super::driver::spawn_worker(updates, async {
        panic!("simulated backend panic");
    });
    let update = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
        .await
        .expect("failure reported")
        .expect("failure event");
    assert!(
        matches!(update, super::Update::Failed(message) if message.contains("simulated backend panic"))
    );
    task.await.expect("supervisor completed");
}
