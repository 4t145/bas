use super::*;
use crate::clipboard::Contents;

#[gpui_kit::test]
fn clipboard_read_blocks_edits_and_send_then_falls_back_as_one_undoable_paste(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("draft", cx);
        let target = fixture.view.update(cx, |view, cx| view.begin_paste(cx));
        window.render_frame(cx);
        window.input("blocked", cx);
        window.press("ctrl-enter", cx);
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "draft"
        );
        assert!(!fixture.view.read(cx).can_send(cx));
        assert!(fixture.view.read(cx).send_hint(cx).starts_with("发送消息"));
        fixture.view.update(cx, |view, cx| {
            view.finish_paste(
                target,
                Err("data-control unavailable".into()),
                Some(ClipboardItem::new_string("中文\r\nnext".into())),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).composer.paste_pending);
        assert!(fixture.view.read(cx).can_send(cx));
        assert!(fixture.view.read(cx).selected.error.is_none());
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "draft中文\nnext"
        );
        window.press("ctrl-z", cx);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "draft"
        );
    })
    .expect("paste window");
}

#[gpui_kit::test]
fn clipboard_result_is_discarded_after_switching_threads_and_switching_back(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries.push(second.clone());
            let target = view.begin_paste(cx);
            view.select(second.id, window, cx);
            view.select(first.id, window, cx);
            view.finish_paste(
                target,
                Ok(Contents {
                    text: Some("late clipboard".into()),
                    ..Contents::default()
                }),
                None,
                window,
                cx,
            );
            assert!(view.composer.input.read(cx).value().is_empty());
            assert!(view.composer.attachments.is_empty());
            assert!(!view.composer.paste_pending);
            assert!(view.selected.error.is_none());
        });
    })
    .expect("switch window");
}

#[gpui_kit::test]
fn clipboard_read_survives_initializing_its_current_draft(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("first draft ", window, cx);
                let end = input.value().len();
                input.set_selected_range(end..end, cx);
            });
            let target = view.begin_paste(cx);
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(session.clone().into()),
                },
                window,
                cx,
            );
            view.finish_paste(
                target,
                Ok(Contents {
                    text: Some("pasted text".into()),
                    ..Contents::default()
                }),
                None,
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(session.id));
            assert_eq!(
                view.composer.input.read(cx).value(),
                "first draft pasted text"
            );
            assert_eq!(
                view.sessions.drafts[&session.id].text,
                "first draft pasted text"
            );
        });
    })
    .expect("initialization window");
}

#[gpui_kit::test]
fn unsupported_clipboard_image_keeps_its_accompanying_text(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let target = view.begin_paste(cx);
            view.finish_paste(
                target,
                Ok(Contents {
                    text: Some("image description".into()),
                    images: vec![gpui_kit::Image::from_bytes(
                        gpui_kit::ImageFormat::Png,
                        b"fixture".to_vec(),
                    )],
                    paths: Vec::new(),
                }),
                None,
                window,
                cx,
            );
            assert_eq!(view.composer.input.read(cx).value(), "image description");
            assert!(view.composer.attachments.is_empty());
            assert!(
                view.selected
                    .error
                    .as_deref()
                    .expect("image capability error")
                    .contains("文本描述图片")
            );
        });
    })
    .expect("mixed clipboard window");
}
