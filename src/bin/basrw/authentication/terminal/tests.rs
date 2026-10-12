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
    assert_eq!(agent_env[std::ffi::OsStr::new(CONTROL_ENV)], None);
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_launcher_exit_is_not_login_completion() {
    let control = control::Listener::bind().expect("control listener");
    let endpoint = control.endpoint().clone();
    let child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "exit 0"])
        .spawn()
        .expect("launcher");
    let login = Login::new(child, control);
    let (connected, ready) = tokio::sync::oneshot::channel();
    let (finish, release) = tokio::sync::oneshot::channel();
    let helper = tokio::spawn(async move {
        let helper = endpoint.connect().await.expect("helper handshake");
        connected.send(()).expect("test waits");
        release.await.expect("completion release");
        helper.finish(0).await.expect("completion");
    });
    let completion = login.complete();
    tokio::pin!(completion);
    tokio::select! {
        _ = ready => {},
        result = &mut completion => panic!("completed before helper: {result:?}"),
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut completion)
            .await
            .is_err(),
        "launcher success cannot mark the backend authenticated"
    );
    finish.send(()).expect("helper waiting");
    tokio::time::timeout(std::time::Duration::from_secs(2), completion)
        .await
        .expect("result feedback")
        .expect("login success");
    helper.await.expect("helper task");
}

#[cfg(unix)]
fn helper_command(
    invocation: &Invocation,
    endpoint: &control::Endpoint,
    hold_failure: bool,
) -> tokio::process::Command {
    let mut command =
        tokio::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "authentication::terminal::tests::helper_process",
            "--nocapture",
        ])
        .env("_BASR_TEST_HELPER_PROCESS", "1")
        .env(
            "_BASR_TEST_HOLD_FAILURE",
            if hold_failure { "1" } else { "0" },
        )
        .env(
            INVOCATION_ENV,
            serde_json::to_string(invocation).expect("invocation"),
        )
        .env(
            CONTROL_ENV,
            serde_json::to_string(endpoint).expect("endpoint"),
        )
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(unix)]
#[test]
fn helper_process() {
    if std::env::var("_BASR_TEST_HELPER_PROCESS").as_deref() != Ok("1") {
        return;
    }
    let invocation = serde_json::from_str::<Invocation>(
        &std::env::var(INVOCATION_ENV).expect("invocation environment"),
    )
    .expect("invocation");
    invocation.validate().expect("valid fixture invocation");
    crate::i18n::set_language(invocation.language);
    let endpoint = serde_json::from_str::<control::Endpoint>(
        &std::env::var(CONTROL_ENV).expect("control environment"),
    )
    .expect("control endpoint");
    let hold = std::env::var("_BASR_TEST_HOLD_FAILURE").as_deref() == Ok("1");
    let code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("helper runtime")
        .block_on(execute_helper(Ok(invocation), endpoint, hold));
    std::process::exit(code);
}

#[cfg(unix)]
#[tokio::test]
async fn helper_reports_the_actual_login_exit_code_and_launch_failure() {
    let directory = tempfile::tempdir().expect("helper feedback directory");
    let shell = which::which("sh").expect("fixture shell");
    for code in [0, 23, 1] {
        let invocation = Invocation {
            language: crate::i18n::Language::Chinese,
            program: if code == 1 {
                directory.path().join("missing-agent")
            } else {
                shell.clone()
            },
            args: vec!["-c".into(), format!("exit {code}")],
            env: BTreeMap::new(),
        };
        let listener = control::Listener::bind().expect("listener");
        let mut child = helper_command(&invocation, listener.endpoint(), false)
            .stdin(Stdio::null())
            .spawn()
            .expect("fixture helper");
        let mut controller =
            tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                .await
                .expect("handshake deadline")
                .expect("helper");
        drop(listener);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), controller.result())
                .await
                .expect("completion deadline")
                .expect("completion"),
            code
        );
        assert_eq!(child.wait().await.expect("helper exits").code(), Some(code));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn closed_terminal_without_helper_feedback_is_reported_promptly() {
    let child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "exit 7"])
        .spawn()
        .expect("launcher fixture");
    let login = Login::new(child, control::Listener::bind().expect("listener"));
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
    use tokio::io::AsyncWriteExt as _;
    let control = control::Listener::bind().expect("listener");
    let invocation = Invocation {
        language: crate::i18n::Language::Chinese,
        program: which::which("sh").expect("fixture shell"),
        args: vec!["-c".into(), "exit 23".into()],
        env: BTreeMap::new(),
    };
    let mut child = helper_command(&invocation, control.endpoint(), true)
        .stdin(Stdio::piped())
        .spawn()
        .expect("terminal awaiting acknowledgement");
    let pid = nix::unistd::Pid::from_raw(child.id().expect("helper pid") as i32);
    let mut acknowledgement = child.stdin.take().expect("held-open terminal input");
    let login = Login::new(child, control);
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), login.complete())
        .await
        .expect("feedback must arrive before the terminal closes")
        .expect_err("helper reported failed login");
    assert!(crate::i18n::visible_text(&error).contains("退出码 23"));
    assert!(error.contains("查看终端后重试"));
    assert!(
        nix::sys::signal::kill(pid, None).is_ok(),
        "completed failure window remains open"
    );
    acknowledgement.write_all(b"\n").await.expect("acknowledge");
    drop(acknowledgement);
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_before_handshake_stops_only_the_unstarted_launcher() {
    let control = control::Listener::bind().expect("listener");
    let endpoint = control.endpoint().clone();
    let mut child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
        .args(["-c", "read line"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("pending launcher");
    let input = child.stdin.take().expect("keep launcher pending");
    let pid = nix::unistd::Pid::from_raw(child.id().expect("launcher pid") as i32);
    drop(Login::new(child, control));
    assert!(
        endpoint.connect().await.is_err(),
        "no authorisation after cancellation"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while nix::sys::signal::kill(pid, None).is_ok() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("unstarted launcher is reaped");
    drop(input);
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_or_timing_out_completion_closes_its_helper_connection() {
    for timeout in [false, true] {
        let control = control::Listener::bind().expect("listener");
        let endpoint = control.endpoint().clone();
        let child = tokio::process::Command::new(which::which("sh").expect("fixture shell"))
            .args(["-c", "exit 0"])
            .spawn()
            .expect("fixture launcher");
        let (connected, ready) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let mut helper = endpoint.connect().await.expect("handshake");
            connected.send(()).expect("test ready");
            tokio::time::timeout(std::time::Duration::from_secs(5), helper.cancelled())
                .await
                .expect("controller drop cancels");
        });
        let limit = if timeout {
            std::time::Duration::from_secs(1)
        } else {
            LOGIN_TIMEOUT
        };
        let mut completion = Box::pin(Login::new(child, control).complete_with_timeout(limit));
        tokio::select! {
            _ = ready => {},
            result = &mut completion => panic!("premature completion: {result:?}"),
        }
        if timeout {
            assert!(completion.await.is_err());
        } else {
            drop(completion);
        }
        peer.await.expect("helper observed cancellation");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn completion_drop_and_deadline_stop_the_authenticated_login_tree() {
    fn running(pid: i32) -> bool {
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err() {
            return false;
        }
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && stat
                .rsplit_once(") ")
                .is_some_and(|(_, state)| state.starts_with('Z'))
        {
            return false;
        }
        true
    }

    for timeout in [false, true] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let root = directory.path().join("root");
        let descendant = directory.path().join("descendant");
        let credential = directory.path().join("credentials");
        std::fs::write(&credential, b"existing fixture credential").expect("fixture credential");
        let invocation = Invocation {
            language: crate::i18n::Language::Chinese,
            program: which::which("sh").expect("fixture shell"),
            args: vec![
                "-c".into(),
                r#"trap '' TERM
printf '%s\n' "$$" > "$ROOT_PID"
sh -c 'trap "" TERM; printf "%s\n" "$$" > "$DESCENDANT_PID"; while :; do sleep 1; done' &
wait"#
                    .into(),
            ],
            env: BTreeMap::from([
                ("ROOT_PID".into(), root.to_string_lossy().into_owned()),
                (
                    "DESCENDANT_PID".into(),
                    descendant.to_string_lossy().into_owned(),
                ),
            ]),
        };
        let control = control::Listener::bind().expect("listener");
        let child = helper_command(&invocation, control.endpoint(), false)
            .stdin(Stdio::null())
            .spawn()
            .expect("authenticated helper");
        let helper_pid = child.id().expect("helper pid") as i32;
        let limit = if timeout {
            std::time::Duration::from_secs(3)
        } else {
            LOGIN_TIMEOUT
        };
        let mut completion = Box::pin(Login::new(child, control).complete_with_timeout(limit));
        let ready = async {
            loop {
                let read = |path: &std::path::Path| {
                    std::fs::read_to_string(path).ok().and_then(|value| {
                        if !value.ends_with('\n') {
                            return None;
                        }
                        value.trim().parse::<i32>().ok()
                    })
                };
                if let (Some(root), Some(descendant)) = (read(&root), read(&descendant)) {
                    break (root, descendant);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let (root_pid, descendant_pid) = tokio::select! {
            pids = tokio::time::timeout(std::time::Duration::from_secs(5), ready) => pids.expect("login starts"),
            result = &mut completion => panic!("login ended before fixture readiness: {result:?}"),
        };
        if timeout {
            assert!(completion.await.is_err());
        } else {
            drop(completion);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while [helper_pid, root_pid, descendant_pid]
                .into_iter()
                .any(running)
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("controller closure stops helper and its login tree");
        assert_eq!(
            std::fs::read(&credential).expect("credential retained"),
            b"existing fixture credential"
        );
    }
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
    let mut invalid = plan.clone();
    invalid
        .env
        .insert(CONTROL_ENV.into(), "spoofed controller".into());
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
