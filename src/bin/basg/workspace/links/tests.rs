use super::*;

fn file_path(target: Target) -> PathBuf {
    match target {
        Target::File { path, .. } => path,
        Target::External(_) => panic!("expected a local file"),
    }
}

#[test]
fn absolute_relative_and_file_uri_links_share_the_same_resolution() {
    let directory = tempfile::tempdir().expect("working directory");
    let path = directory.path().join("notes 中文.txt");
    let context = Context::new(Some(directory.path().to_owned()), true);
    assert_eq!(
        file_path(
            context
                .target(&path.to_string_lossy())
                .expect("absolute path")
        ),
        path
    );
    assert_eq!(
        file_path(
            context
                .target("notes%20%E4%B8%AD%E6%96%87.txt")
                .expect("relative path")
        ),
        path
    );
    assert_eq!(
        file_path(
            context
                .target(Url::from_file_path(&path).expect("file URL").as_str())
                .expect("file URI")
        ),
        path
    );
    assert!(matches!(
        Context::new(None, true).target("notes.txt"),
        Err(Error::MissingDirectory)
    ));
}

#[test]
fn remote_paths_and_unsupported_schemes_never_launch_local_resources() {
    let remote = Context::default();
    for address in ["file:///tmp/notes.txt", "/tmp/notes.txt", "notes.txt"] {
        assert!(remote.target(address).is_err(), "{address}");
    }
    for address in [
        "",
        "#section",
        "?query",
        "javascript:alert(1)",
        "data:text/html,hello",
        "mcp://resource",
    ] {
        assert!(
            Context::new(None, true).target(address).is_err(),
            "{address}"
        );
    }
    for address in [
        "https://example.com/file#part",
        "http://example.com",
        "mailto:test@example.com",
    ] {
        assert!(matches!(remote.target(address), Ok(Target::External(_))));
    }
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn file_link_clicks_open_the_target_and_missing_files_show_an_error(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{AppContext, test::TestWindowExt};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let directory = tempfile::tempdir().expect("artifacts");
    let path = directory.path().join("image.png");
    std::fs::write(&path, b"fixture").expect("existing artifact");
    let context = Context::new(Some(directory.path().to_owned()), true);
    cx.update(|cx| context.open(&path.to_string_lossy(), cx));
    assert_eq!(
        cx.opened_url().as_deref(),
        Some(Url::from_file_path(&path).expect("URI").as_str())
    );
    let popup = cx.update(|cx| {
        let before = cx.windows();
        context.open("missing.txt", cx);
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .expect("error feedback")
    });
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("application-message-body")
                .label()
                .expect("error text")
                .contains("missing.txt")
        );
    })
    .expect("visible file error");
}
