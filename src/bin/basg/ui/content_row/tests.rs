use super::*;
use gpui_kit::{AppContext, Context, Render, TestAppContext, px, size, test::TestWindowExt};

struct Rows;

impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let description = div()
            .truncate()
            .child("Long resource description ".repeat(20));
        ContentRow::new("row", description)
            .icon(IconName::File)
            .child(crate::ui::icon_action(
                "open",
                IconName::ExternalLink,
                "Open",
            ))
            .child(crate::ui::icon_action("copy", IconName::Copy, "Copy"))
    }
}

#[gpui_kit::test]
fn content_yields_space_to_fixed_size_actions(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let window = cx.open_window(size(px(320.), px(100.)), |_, _| Rows);
    cx.update_window(window.into(), |_, window, cx| {
        for width in [320., 200.] {
            window.resize(size(px(width), px(100.)));
            window.render_frame(cx);
            let row = window.find("row").bounds();
            let body = window.within("row").find("content-row-body").bounds();
            let actions = window.within("row").find("content-row-actions").bounds();
            assert!(body.right() <= actions.left());
            assert!(actions.right() <= row.right());
            for id in ["open", "copy"] {
                let button = window.find(id).bounds();
                assert_eq!(
                    button.size,
                    size(
                        px(crate::ui::COMPACT_CONTROL),
                        px(crate::ui::COMPACT_CONTROL)
                    )
                );
                assert_eq!(button.center().y, row.center().y);
            }
        }
    })
    .expect("shared content row geometry");
}
