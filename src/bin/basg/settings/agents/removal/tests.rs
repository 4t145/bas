use super::*;
use gpui_kit::{TestAppContext, px, size, test::TestWindowExt};

#[gpui_kit::test]
fn removal_cancel_and_stale_confirmation_preserve_the_saved_registration(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let store = Store::at(directory.path().join("settings.json"));
    let original = AgentSpec::acp("custom", Command::new("/fixture/original", []));
    let replacement = AgentSpec::acp("custom", Command::new("/fixture/replacement", []));
    let mut document = store.load().expect("document");
    store
        .save_agents(
            &mut document,
            vec![AgentSpec::embedded().into(), original.into()],
        )
        .expect("original");
    cx.update(super::super::tests::init);
    let mut agents = None;
    let configuration = cx.open_window(size(px(900.), px(720.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store.clone(), window, cx));
        agents = Some(page.clone());
        Root::new(page, window, cx)
    });
    let agents = agents.expect("page");
    for cancel in [true, false] {
        let confirmation = cx
            .update_window(configuration.into(), |_, window, cx| {
                window.render_frame(cx);
                let before = cx.windows();
                window.click("agents-remove-custom", cx);
                cx.windows()
                    .into_iter()
                    .find(|window| !before.contains(window))
                    .expect("confirmation")
            })
            .expect("request removal");
        if !cancel {
            store
                .save_agents(
                    &mut document,
                    vec![AgentSpec::embedded().into(), replacement.clone().into()],
                )
                .expect("external replacement while confirmation is open");
        }
        let before = std::fs::read(store.path()).expect("before response");
        cx.update_window(confirmation, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("delete-dont-ask").is_none(),
                "session deletion preferences do not bypass agent removal confirmation"
            );
            window.click(
                if cancel {
                    "message-close"
                } else {
                    "message-confirm"
                },
                cx,
            );
        })
        .expect("answer confirmation");
        assert_eq!(std::fs::read(store.path()).expect("after response"), before);
    }
    cx.update(|cx| {
        assert!(matches!(agents.read(cx).editor, Editor::Idle));
        assert_eq!(agents.read(cx).message, DraftError::Changed.to_string());
    });
}
