use gpui_kit::base::text::{CodeBlock, TableData};
use gpui_kit::{AvailableSpace, canvas};

use super::*;

const CODE_GROUP: &str = "markdown-code-block";
const TABLE_GROUP: &str = "markdown-table-actions";

pub(super) fn code(block: &CodeBlock, _: &mut Window, _: &mut App) -> AnyElement {
    let source = block.code().to_string();
    let start = block.span.map_or(0, |span| span.start);
    // The code style owns an overflow clip, so this mask is the visible part
    // of this code block, including its padding. Keep the native text tree
    // intact: replacing blocks with nested TextViews breaks range selection.
    canvas(
        move |anchor, window, cx| {
            let region = window.content_mask().bounds;
            let mut actions = div()
                .id((CODE_GROUP, start))
                .test_support()
                .group(CODE_GROUP)
                .w(region.size.width)
                .h(region.size.height)
                .relative()
                .child(
                    copy_button(
                        "copy-markdown-code",
                        crate::i18n::tr!("action-copy-code"),
                        source,
                        cx,
                    )
                    .absolute()
                    .left(anchor.left() - region.left() - px(crate::ui::COMPACT_CONTROL))
                    .top(anchor.top() - region.top())
                    .invisible()
                    .group_hover(CODE_GROUP, |style| style.visible()),
                )
                .into_any_element();
            actions.prepaint_as_root(
                region.origin,
                region.size.map(AvailableSpace::Definite),
                window,
                cx,
            );
            actions
        },
        |_, mut actions, window, cx| actions.paint(window, cx),
    )
    // The upstream absolute toolbar paints its own background. Its layout
    // stays empty; the local overlay paints only the hovered block's button.
    .size(px(0.))
    .into_any_element()
}

pub(super) fn table(table: &TableData, _: &mut Window, cx: &mut App) -> AnyElement {
    div()
        .id(TABLE_GROUP)
        .test_support()
        .group(TABLE_GROUP)
        .flex()
        .justify_end()
        .child(
            copy_button(
                "copy-markdown-table",
                crate::i18n::tr!("action-copy-table"),
                table.markdown.clone(),
                cx,
            )
            .invisible()
            .group_hover(TABLE_GROUP, |style| style.visible()),
        )
        .into_any_element()
}

fn copy_button(id: &'static str, label: &'static str, source: String, cx: &App) -> Button {
    message_actions::action_button(id, IconName::Copy, cx)
        .accessibility_label(label)
        .tooltip(label)
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(source.clone()));
        })
}
