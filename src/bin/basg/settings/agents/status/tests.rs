use super::*;

#[test]
fn availability_probe_login_and_previous_results_have_separate_labels() {
    assert_eq!(label(&Status::Installed("/tool".into())), "已安装 · 待检测");
    assert_eq!(check_label(&Status::Installed("/tool".into())), "检测");
    assert_eq!(label(&Status::Connecting), "正在检测…");
    assert_eq!(label(&Status::Connected("/tool".into())), "检测通过");
    assert_eq!(check_label(&Status::Connected("/tool".into())), "重新检测");
    let login = Status::AuthenticationRequired {
        program: "/tool".into(),
        methods: vec![],
    };
    assert_eq!(label(&login), "需要认证");
    assert_eq!(check_label(&login), "重新检测");
    assert!(Status::Connecting.busy());
    assert!(Status::Authenticating.busy());
    assert!(!login.busy());
}
