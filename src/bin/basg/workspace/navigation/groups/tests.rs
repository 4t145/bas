use super::*;
use bas::model::{Owner, SessionId};

fn thread(path: Option<&str>, title: &str) -> SessionMeta {
    let mut session = SessionMeta::new(SessionId::new(), "pi".into(), Owner::current());
    session.cwd = path.map(PathBuf::from);
    session.title = Some(title.into());
    session
}

#[test]
fn groups_by_full_daemon_path_and_keeps_each_threads_order() {
    let sessions = [
        thread(Some("/one/app"), "recent"),
        thread(Some("/two/app"), "other"),
        thread(Some("/one/./app/"), "older"),
        thread(None, "default"),
    ];
    let groups = group_threads(&sessions, &[PathBuf::from("/empty")], "");
    assert_eq!(groups.len(), 4);
    assert_eq!(groups[0].threads.len(), 2);
    assert_eq!(groups[0].threads[0].title.as_deref(), Some("recent"));
    assert_eq!(groups[0].threads[1].title.as_deref(), Some("older"));
    assert_eq!(groups[1].directory.as_deref(), Some(Path::new("/two/app")));
    assert!(groups[2].directory.is_none());
    assert!(groups[3].threads.is_empty());
}
