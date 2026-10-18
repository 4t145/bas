use super::*;
use crate::workspace::links as resource_links;
use gpui_kit::{Modifiers, VisualTestContext, point};

struct LinkMarkdown {
    state: Entity<TextViewState>,
    links: resource_links::Context,
}

impl Render for LinkMarkdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        view(&self.state, cx).links(self.links.clone())
    }
}

#[gpui_kit::test]
fn rendered_markdown_file_links_use_the_conversations_working_directory(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
    });
    let directory = tempfile::tempdir().expect("artifacts");
    let path = directory.path().join("prompt 中文.txt");
    std::fs::write(&path, "artifact").expect("prompt file");
    let uri = url::Url::from_file_path(&path).expect("file URI");
    for address in [
        path.to_string_lossy().replace('\\', "/"),
        "prompt%20%E4%B8%AD%E6%96%87.txt".into(),
        uri.to_string(),
    ] {
        let state =
            cx.new(|cx| TextViewState::markdown(&format!("[open artifact](<{address}>)"), cx));
        let window = cx.open_window(size(px(320.), px(160.)), |_, _| LinkMarkdown {
            state: state.clone(),
            links: resource_links::Context::new(Some(directory.path().to_owned()), true),
        });
        cx.run_until_parked();
        let position = cx
            .update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                state.read(cx).bounds().origin + point(px(5.), px(10.))
            })
            .expect("render file link");
        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            visual.simulate_click(position, Modifiers::default());
        }
        assert_eq!(cx.opened_url().as_deref(), Some(uri.as_str()), "{address}");
        cx.update(|cx| cx.open_url("about:blank"));
        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            visual.simulate_mouse_down(
                position,
                gpui_kit::MouseButton::Right,
                Modifiers::default(),
            );
            visual.simulate_mouse_up(position, gpui_kit::MouseButton::Right, Modifiers::default());
        }
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("about:blank"),
            "a context click does not open the file"
        );
    }
}
