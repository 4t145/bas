use super::*;

#[test]
fn availability_probe_login_and_previous_results_have_separate_labels() {
    assert_eq!(label(&Status::Installed("/tool".into())), "已安装 · 未验证");
    assert_eq!(
        check_label(&Status::Installed("/tool".into())),
        "验证可用性"
    );
    assert_eq!(label(&Status::Checking), "正在验证程序…");
    assert_eq!(label(&Status::Connecting), "正在验证可用性…");
    assert_eq!(label(&Status::Connected("/tool".into())), "可用");
    assert_eq!(
        check_label(&Status::Connected("/tool".into())),
        "重新验证可用性"
    );
    let login = Status::AuthenticationRequired {
        program: "/tool".into(),
        methods: vec![],
    };
    assert_eq!(label(&login), "需要认证");
    assert_eq!(check_label(&login), "重新验证可用性");
    assert!(Status::Connecting.busy());
    assert!(Status::Authenticating.busy());
    assert!(!login.busy());
}

#[test]
fn completed_installation_uses_the_stable_installed_label() {
    assert_eq!(
        crate::i18n::tr!("agent-install-done"),
        crate::i18n::tr!("agent-installed")
    );
    assert_eq!(crate::i18n::tr!("agent-install-done"), "已安装");
    assert_ne!(
        label(&Status::Installed("/tool".into())),
        label(&Status::Connected("/tool".into()))
    );
}
