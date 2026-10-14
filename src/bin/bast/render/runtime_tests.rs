use super::*;

#[test]
fn runtime_failure_overrides_committed_status_and_escapes_error() {
    let mut meta = model::SessionMeta::new(
        model::SessionId::new(),
        "test".into(),
        model::Owner::current(),
    );
    meta.status = model::SessionStatus::Working;
    meta.revision = model::Cursor::from_raw(7);
    let mut snapshot = model::SessionSnapshot::from(meta);
    snapshot.runtime.failure = Some(model::RuntimeFailure::Persistence {
        message: "disk\x1b]52;c;clipboard\x07\nfull".into(),
    });
    let session = pb::Session::try_from(&snapshot).expect("snapshot");
    let mut output = Vec::new();
    write_session(&session, &mut output).expect("show");
    let output = String::from_utf8(output).expect("UTF-8");
    assert!(output.contains("status    failed"));
    assert!(output.contains("committed working (revision 7)"));
    assert!(output.contains("\\u001b]52;c;clipboard\\u0007\nfull"));
    assert!(!output.contains('\x1b'));
    let mut output = Vec::new();
    write_sessions(&[session], &mut output).expect("list");
    let output = String::from_utf8(output).expect("UTF-8");
    assert!(output.contains("failed"));
    assert!(!output.contains("working"));
}
