use bas::acp;
use std::sync::Arc;

use super::{
    AgentSpec, Catalog, CatalogError, Command, Host, Kind, LocalHost, PermissionKind,
    PermissionOutcome, PermissionPolicy, PermissionRequest,
};
use bas::model::{AgentId, SessionId, TurnId};

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
            cwd: std::env::current_dir().expect("working directory exists"),
            host: Arc::new(LocalHost::default()),
            updates: tokio::sync::mpsc::channel(16).0,
        })
        .await
        .err()
        .expect("the backend is not configured");

    assert!(error.to_string().contains("not configured"));
}

#[tokio::test]
async fn embedded_restore_reuses_public_identity_and_can_prompt() {
    use super::{TurnUpdate, Update, driver::Driver as _};

    let id = SessionId::new();
    let cwd = std::env::current_dir().expect("working directory exists");
    let (updates, mut inbox) = tokio::sync::mpsc::channel(16);
    let mut agent = super::embedded::Driver::new()
        .restore(
            super::LaunchRequest {
                session: id,
                spec: Arc::new(AgentSpec::embedded()),
                cwd: cwd.clone(),
                host: Arc::new(LocalHost::default()),
                updates,
            },
            String::new(),
        )
        .await
        .expect("stateless echo can reopen");
    assert!(agent.acp_session_id().is_none());
    let turn = TurnId::new();
    agent
        .prompt(
            turn,
            acp::PromptRequest::new("ignored", vec!["after restart".into()]),
        )
        .expect("echo accepts prompt");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut reply = String::new();
        loop {
            match inbox.recv().await.expect("update") {
                Update::Turn {
                    turn: actual,
                    update: TurnUpdate::Acp(acp::SessionUpdate::AgentMessageChunk(chunk)),
                } => {
                    assert_eq!(actual, turn);
                    let acp::ContentBlock::Text(text) = chunk.content else {
                        panic!("expected text echo");
                    };
                    reply.push_str(&text.text);
                }
                Update::Turn {
                    turn: actual,
                    update: TurnUpdate::Finished(Ok(_)),
                } => {
                    assert_eq!(actual, turn);
                    break reply;
                }
                other => panic!("unexpected echo update: {other:?}"),
            }
        }
    })
    .await
    .expect("echo replies");
    assert!(reply.contains(&id.to_string()));
    assert!(reply.contains(&cwd.display().to_string()));
    assert!(reply.contains("after restart"));
    agent.shutdown().await;
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

#[test]
fn catalog_lists_configured_backends() {
    let catalog = Catalog::with_defaults();

    assert_eq!(
        catalog.spec(&AgentId::new("embedded")).unwrap().kind,
        Kind::Embedded
    );
    assert_eq!(catalog.specs().count(), 1);
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

#[tokio::test]
async fn local_host_applies_limit_without_an_explicit_start_line() {
    let directory = tempfile::tempdir().expect("temporary workspace");
    let path = directory.path().join("lines.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").expect("fixture file");
    let text = LocalHost::default()
        .read_text_file(super::FileReadRequest::new("test", path).limit(2))
        .await
        .expect("read succeeds");
    assert_eq!(text, "one\ntwo");
}

#[test]
fn command_and_spec_parsers_reject_empty_inputs() {
    for input in ["", " \t\n"] {
        assert!(input.parse::<super::Command>().is_err());
        assert!(input.parse::<super::AgentSpec>().is_err());
    }
    let command: super::Command = " agent --mode acp "
        .parse()
        .expect("command with arguments");
    assert_eq!(command.program, std::path::PathBuf::from("agent"));
    assert_eq!(command.args, ["--mode", "acp"]);
    let spec: super::AgentSpec = " custom=acp:agent --mode acp ".parse().expect("agent spec");
    assert_eq!(spec.id.as_str(), "custom");
    assert_eq!(spec.command, Some(command));
}
