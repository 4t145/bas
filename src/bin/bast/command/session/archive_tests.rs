use super::*;

#[test]
fn old_daemon_has_actionable_upgrade_error_not_delete_fallback() {
    let error = archive_error(tonic::Status::unimplemented("unknown method")).to_string();
    assert!(error.contains("upgrade bas and retry"));
    assert!(error.contains("do not use session remove"));
    assert!(error.contains("permanently"));
}

#[test]
fn other_rpc_errors_keep_their_status_and_context() {
    for status in [
        tonic::Status::not_found("missing session"),
        tonic::Status::unavailable("offline"),
    ] {
        let code = status.code();
        let error = archive_error(status);
        assert_eq!(
            error
                .downcast_ref::<tonic::Status>()
                .expect("RPC status")
                .code(),
            code
        );
        assert!(
            error
                .to_string()
                .contains("set session archive state failed")
        );
    }
}
