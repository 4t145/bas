use super::*;
use basr::config::agent::Command;

fn spec() -> AgentSpec {
    AgentSpec::acp("tool", Command::new("/configured/agent", ["--base".into()]))
}

#[test]
fn terminal_auth_appends_arguments_and_overrides_env_without_interpreting_meta() {
    let mut spec = spec();
    spec.env.insert("KEEP".into(), "base".into());
    spec.env.insert("OVERRIDE".into(), "base".into());
    let method: acp::AuthMethod = serde_json::from_value(serde_json::json!({
        "type": "terminal",
        "id": "login",
        "name": "Log in",
        "args": ["--login", "a b", "$(not-a-shell)"],
        "env": {"OVERRIDE":"method", "PRIVATE":"fixture-secret"},
        "_meta": {"terminal-auth":{"command":"/untrusted/other-command","args":["--bad"]}}
    }))
    .expect("terminal auth schema");
    let invocation = Invocation::new(&spec, "/resolved/agent", &method).expect("invocation");
    assert_eq!(invocation.program, PathBuf::from("/resolved/agent"));
    assert_eq!(
        invocation.args,
        ["--base", "--login", "a b", "$(not-a-shell)"]
    );
    assert_eq!(invocation.env["KEEP"], "base");
    assert_eq!(invocation.env["OVERRIDE"], "method");
    let notice = invocation.confirmation();
    assert!(!notice.contains("fixture-secret"));
    assert!(!notice.contains("/untrusted/other-command"));
}

#[test]
fn nonterminal_methods_and_unchecked_programs_are_not_executed() {
    let agent = acp::AuthMethod::Agent(acp::AuthMethodAgent::new("login", "Login"));
    assert!(matches!(
        Invocation::new(&spec(), "/resolved/agent", &agent),
        Err(Error::Unsupported)
    ));
    let terminal = acp::AuthMethod::Terminal(acp::AuthMethodTerminal::new("login", "Login"));
    assert!(matches!(
        Invocation::new(&spec(), "relative", &terminal),
        Err(Error::MissingProgram)
    ));
    let invalid = acp::AuthMethod::Terminal(
        acp::AuthMethodTerminal::new("login", "Login").args(vec!["bad\0argument".into()]),
    );
    assert!(matches!(
        Invocation::new(&spec(), "/resolved/agent", &invalid),
        Err(Error::InvalidCommand)
    ));
}

#[cfg(all(unix, feature = "gui-tests"))]
#[gpui_kit::test]
fn auth_required_is_actionable_without_drafting_or_automatically_launching(
    cx: &mut gpui_kit::TestAppContext,
) {
    use super::super::Check;
    use gpui_kit::{px, size, test::TestWindowExt};
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    let store = basr::config::Store::at(path.clone());
    let mut document = store.load().expect("settings");
    let spec = spec();
    store
        .save_agents(&mut document, vec![spec.clone()])
        .expect("catalog");
    let before = std::fs::read(&path).expect("settings bytes");
    let method = acp::AuthMethod::Terminal(
        acp::AuthMethodTerminal::new("login", "Configure provider")
            .args(vec!["--terminal-login".into(), "long-path/".repeat(200)]),
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store.clone(), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            page.checks.insert(
                spec.id.clone(),
                Check {
                    spec: spec.clone(),
                    status: check::Status::AuthenticationRequired {
                        program: "/resolved/agent".into(),
                        methods: vec![method.clone()],
                    },
                    _task: None,
                },
            );
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("agents-authentication-0").visible());
        assert!(window.find("agents-authenticate-0-0").visible());
        assert!(
            view.read(cx).message.is_empty(),
            "no terminal launch happened before consent"
        );
        assert!(view.read(cx).draft.is_none());
        window.click("agents-authenticate-0-0", cx);
    })
    .expect("request confirmation without launching");
    let confirmation = cx.update(|cx| {
        let mut windows = cx
            .windows()
            .into_iter()
            .filter(|other| other.window_id() != window.window_id());
        windows.next().expect("confirmation window")
    });
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("message-confirm").label(), Some("打开认证终端"));
        let confirm = window.find("message-confirm").bounds();
        assert!(confirm.bottom() <= window.viewport_size().height);
        assert!(window.find("application-message-body").bounds().bottom() <= confirm.top());
        window.click("message-close", cx);
    })
    .expect("cancel consent");
    cx.update(|cx| {
        assert!(
            view.read(cx).message.is_empty(),
            "cancelling consent does not launch a terminal"
        );
        assert_eq!(std::fs::read(&path).expect("unchanged"), before);
    });
    // A later file edit invalidates the checked method even if the UI retained it.
    let mut changed = store.load().expect("load");
    let mut changed_spec = spec.clone();
    changed_spec.command = Some(Command::new("/changed/agent", []));
    store
        .save_agents(&mut changed, vec![changed_spec])
        .expect("external edit");
    cx.update(|cx| {
        let error = view
            .read(cx)
            .authentication_invocation(&spec, &method, cx)
            .err()
            .expect("stale command rejected");
        assert!(error.contains("配置已变化"));
    });
}

#[cfg(unix)]
#[test]
fn target_invocation_keeps_argument_boundaries() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let output = directory.path().join("arguments");
    let shell = which::which("sh").expect("shell for fixture only");
    let invocation = Invocation {
        program: shell.clone(),
        args: vec![
            "-c".into(),
            "printf '%s\\n' \"$1\" \"$2\" > \"$OUT\"".into(),
            "fixture".into(),
            "a b".into(),
            "$(literal)".into(),
        ],
        env: BTreeMap::from([("OUT".into(), output.to_string_lossy().into_owned())]),
    };
    assert!(
        invocation
            .agent_command()
            .status()
            .expect("fake login")
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(output).expect("argv"),
        "a b\n$(literal)\n"
    );
}

#[test]
fn agent_environment_is_inert_in_the_launcher_and_removed_from_target_payload() {
    let invocation = Invocation {
        program: "/configured/agent".into(),
        args: vec!["--login".into()],
        env: BTreeMap::from([
            ("LD_PRELOAD".into(), "/fixture/loader".into()),
            ("PATH".into(), "/fixture/path".into()),
            ("PRIVATE".into(), "fixture-secret".into()),
        ]),
    };
    let command = terminal_command(
        &invocation,
        Terminal {
            program: "/terminal".into(),
            prefix: &["-e"],
        },
        "/basrw".into(),
    )
    .expect("launcher command");
    let env: Vec<_> = command.as_std().get_envs().collect();
    assert_eq!(env.len(), 1);
    assert_eq!(env[0].0, INVOCATION_ENV);
    assert!(
        !command
            .as_std()
            .get_args()
            .any(|arg| arg.to_string_lossy().contains("fixture-secret"))
    );
    let decoded: Invocation =
        serde_json::from_str(env[0].1.expect("payload").to_str().expect("JSON"))
            .expect("invocation");
    let agent_command = decoded.agent_command();
    let agent_env: std::collections::HashMap<_, _> = agent_command.get_envs().collect();
    assert_eq!(
        agent_env[std::ffi::OsStr::new("PATH")],
        Some(std::ffi::OsStr::new("/fixture/path"))
    );
    assert_eq!(agent_env[std::ffi::OsStr::new(INVOCATION_ENV)], None);
}
