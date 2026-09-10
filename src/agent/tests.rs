use std::sync::Arc;

use super::{
    AgentSpec, Catalog, CatalogError, Command, Host, Kind, LocalHost, PermissionKind,
    PermissionOption, PermissionOutcome, PermissionPolicy, PermissionRequest,
};
use crate::model::{AgentId, SessionId, ToolCall, ToolCallId};

#[test]
fn parses_agent_kinds() {
    assert_eq!("embedded".parse::<Kind>().unwrap(), Kind::Embedded);
    assert_eq!(" ACP ".parse::<Kind>().unwrap(), Kind::Acp);
    assert!("mcp".parse::<Kind>().is_err());
}

#[test]
fn command_renders_a_shell_line() {
    let command = Command::new("/usr/bin/opencode", ["acp".to_owned()]);

    assert_eq!(command.to_string(), "/usr/bin/opencode acp");
}

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

#[test]
fn catalog_rejects_a_driver_of_the_wrong_kind() {
    let mut catalog = Catalog::new();
    let error = catalog
        .insert(AgentSpec::embedded(), Arc::new(super::acp::Driver::new()))
        .unwrap_err();

    assert!(matches!(error, CatalogError::KindMismatch { .. }));
}

#[tokio::test]
async fn catalog_reports_unknown_backends() {
    let catalog = Catalog::with_defaults();
    let error = catalog
        .launch(super::LaunchRequest {
            session: SessionId::new(),
            spec: Arc::new(AgentSpec::acp("missing", Command::new("nope", Vec::new()))),
            owner: crate::model::Owner::current(),
            cwd: None,
            title: None,
            host: Arc::new(LocalHost::default()),
            updates: tokio::sync::mpsc::unbounded_channel().0,
        })
        .await
        .err()
        .expect("the backend is not configured");

    assert!(error.to_string().contains("not configured"));
}

#[tokio::test]
async fn local_host_follows_its_permission_policy() {
    let request = PermissionRequest {
        session: SessionId::new(),
        call: ToolCall::new(ToolCallId::new("call-1"), "write a file"),
        options: vec![
            PermissionOption {
                id: "allow".to_owned(),
                label: "Allow once".to_owned(),
                kind: PermissionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject".to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionKind::RejectOnce,
            },
        ],
    };

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
        PermissionOutcome::Selected(PermissionOption {
            id: "allow".to_owned(),
            label: "Allow once".to_owned(),
            kind: PermissionKind::AllowOnce,
        })
    );
    assert_eq!(
        denied,
        PermissionOutcome::Selected(PermissionOption {
            id: "reject".to_owned(),
            label: "Reject".to_owned(),
            kind: PermissionKind::RejectOnce,
        })
    );
}

#[test]
fn catalog_lists_configured_backends() {
    let catalog = Catalog::with_defaults();

    assert_eq!(
        catalog.spec(&AgentId::new("embedded")).unwrap().kind,
        Kind::Embedded
    );
    assert_eq!(catalog.specs().count(), 1);
}
