use super::*;
#[cfg(unix)]
#[test]
fn target_invocation_keeps_argument_boundaries() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let output = directory.path().join("arguments");
    let shell = which::which("sh").expect("shell for fixture only");
    let invocation = Invocation {
        language: crate::i18n::Language::Chinese,
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
        language: crate::i18n::Language::English,
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
    assert_eq!(decoded.language, crate::i18n::Language::English);
    let agent_command = decoded.agent_command();
    let agent_env: std::collections::HashMap<_, _> = agent_command.get_envs().collect();
    assert_eq!(
        agent_env[std::ffi::OsStr::new("PATH")],
        Some(std::ffi::OsStr::new("/fixture/path"))
    );
    assert_eq!(agent_env[std::ffi::OsStr::new(INVOCATION_ENV)], None);
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_launcher_exit_and_incomplete_feedback_are_not_login_completion() {
    let directory = tempfile::tempdir().expect("login result directory");
    let result = directory.path().join("result");
    std::fs::write(&result, "").expect("incomplete helper feedback");
    let child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "exit 0"])
        .spawn()
        .expect("launcher");
    let login = Login {
        child,
        result: result.clone(),
        _directory: directory,
    };
    let completion = login.complete();
    tokio::pin!(completion);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut completion)
            .await
            .is_err(),
        "launcher success and incomplete feedback cannot mark the backend authenticated"
    );
    tokio::fs::write(result, "0")
        .await
        .expect("helper completion");
    tokio::time::timeout(std::time::Duration::from_secs(2), completion)
        .await
        .expect("result feedback")
        .expect("login success");
}

#[cfg(unix)]
#[test]
fn helper_reports_the_actual_login_exit_code_and_launch_failure() {
    let directory = tempfile::tempdir().expect("helper feedback directory");
    let result = directory.path().join("result");
    let shell = which::which("sh").expect("fixture shell");
    for code in [0, 23] {
        let invocation = Invocation {
            language: crate::i18n::Language::Chinese,
            program: shell.clone(),
            args: vec!["-c".into(), format!("exit {code}")],
            env: BTreeMap::new(),
        };
        assert_eq!(execute_helper(Ok(invocation), Some(&result), false), code);
        assert_eq!(
            std::fs::read_to_string(&result).expect("feedback"),
            code.to_string()
        );
    }
    let missing = Invocation {
        language: crate::i18n::Language::Chinese,
        program: directory.path().join("missing-agent"),
        args: vec![],
        env: BTreeMap::new(),
    };
    assert_eq!(execute_helper(Ok(missing), Some(&result), false), 1);
    assert_eq!(
        std::fs::read_to_string(result).expect("launch feedback"),
        "1"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn closed_terminal_without_helper_feedback_is_reported_promptly() {
    let directory = tempfile::tempdir().expect("login result directory");
    let child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "exit 7"])
        .spawn()
        .expect("launcher fixture");
    let login = Login {
        child,
        result: directory.path().join("result"),
        _directory: directory,
    };
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), login.complete())
        .await
        .expect("terminal exit must not wait for login timeout")
        .expect_err("terminal failed before starting login");
    assert!(crate::i18n::visible_text(&error).contains("退出码 7"));
    assert!(error.contains("请重新打开"));
}

#[cfg(unix)]
#[tokio::test]
async fn helper_failure_is_reported_while_terminal_remains_open() {
    let directory = tempfile::tempdir().expect("login result directory");
    let result = directory.path().join("result");
    let mut child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "read line"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("terminal awaiting acknowledgement");
    let acknowledgement = child.stdin.take().expect("held-open terminal input");
    std::fs::write(&result, "23").expect("helper result");
    let login = Login {
        child,
        result,
        _directory: directory,
    };
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), login.complete())
        .await
        .expect("feedback must arrive before the terminal closes")
        .expect_err("helper reported failed login");
    assert!(crate::i18n::visible_text(&error).contains("退出码 23"));
    assert!(error.contains("查看终端后重试"));
    drop(acknowledgement);
}

#[test]
fn daemon_plan_is_used_verbatim_and_invalid_helper_transport_is_rejected() {
    let plan = basr::authentication::Invocation {
        program: "/resolved/launcher".into(),
        args: vec!["/installed/agent.js".into(), "login".into(), "a b".into()],
        env: BTreeMap::from([("KEEP".into(), "value".into())]),
    };
    let invocation = Invocation::from_plan(plan.clone()).expect("validated plan");
    assert_eq!(invocation.program, plan.program);
    assert_eq!(invocation.args, plan.args);
    assert_eq!(invocation.env, plan.env);
    let mut invalid = plan.clone();
    invalid
        .env
        .insert(INVOCATION_ENV.into(), "spoofed helper payload".into());
    assert!(matches!(
        Invocation::from_plan(invalid),
        Err(Error::InvalidCommand)
    ));
    let mut invalid = plan;
    invalid.program = "relative".into();
    assert!(matches!(
        Invocation::from_plan(invalid),
        Err(Error::MissingProgram)
    ));
}
