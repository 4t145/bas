use super::*;
use gpui_kit::{TestAppContext, size, test::TestWindowExt};
use url::Url;

struct Card {
    uri: String,
    local: bool,
}

impl Render for Card {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gds_uim::el! {
            #div { p_4; w_full; } [
                $super::render("sample", &bas::acp::ResourceLink::new("sample.md", self.uri.clone()), &links::Context::new(None, self.local), cx)
            ]
        }
    }
}

#[gpui_kit::test]
fn resource_cards_open_real_links_and_copy_without_opening(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let directory = tempfile::tempdir().expect("resources");
    let path = directory.path().join("sample 中文.md");
    std::fs::write(&path, "sample").expect("local resource");
    let file_uri = Url::from_file_path(path).expect("file URI").to_string();
    for uri in ["https://example.com/sample.md".to_owned(), file_uri] {
        let window = cx.open_window(size(px(560.), px(180.)), |window, cx| {
            let card = cx.new(|_| Card {
                uri: uri.clone(),
                local: true,
            });
            Root::new(card, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            cx.open_url("about:blank");
            let open = window.find("open-resource-sample").bounds();
            assert_eq!(
                open.size,
                size(
                    px(crate::ui::COMPACT_CONTROL),
                    px(crate::ui::COMPACT_CONTROL)
                )
            );
            assert!(window.find("resource-uri-sample").bounds().right() < open.left());
            window.click("resource-name-sample", cx);
            window.click("resource-uri-sample", cx);
        })
        .expect("passive resource description");
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("about:blank"),
            "only the open button launches a resource"
        );
        cx.update_window(window.into(), |_, window, cx| {
            assert_eq!(
                window.find("open-resource-sample").role(),
                Some(gpui_kit::Role::Button)
            );
            window.click("open-resource-sample", cx);
        })
        .expect("open resource");
        assert_eq!(cx.opened_url().as_deref(), Some(uri.as_str()));
        cx.update_window(window.into(), |_, window, cx| {
            cx.open_url("about:blank");
            window.hover("resource-sample", cx);
            window.click("sample", cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(uri.clone())
            );
        })
        .expect("copy resource URI");
        assert_eq!(cx.opened_url().as_deref(), Some("about:blank"));
    }
}

#[test]
fn remote_files_and_non_openable_schemes_are_not_local_launch_targets() {
    assert!(
        links::Context::default()
            .target("file:///tmp/sample.md")
            .is_err()
    );
    for uri in [
        "javascript:alert(1)",
        "data:text/html,<script>",
        "mcp://resource/one",
        "not a URI",
    ] {
        assert!(
            links::Context::new(None, true).target(uri).is_err(),
            "{uri}"
        );
    }
    assert!(
        links::Context::default()
            .target("https://example.com/resource")
            .is_ok()
    );
}

#[gpui_kit::test]
fn missing_resource_files_report_the_problem_without_launching(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let directory = tempfile::tempdir().expect("resources");
    let uri = Url::from_file_path(directory.path().join("missing.md")).expect("file URI");
    let message = cx.update(|cx| {
        let before = cx.windows();
        links::Context::new(None, true).open(uri.as_str(), cx);
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .expect("error window")
    });
    assert!(cx.opened_url().is_none());
    cx.update_window(message, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("application-message-body")
                .label()
                .expect("error")
                .contains("missing.md")
        );
    })
    .expect("missing file feedback");
}
