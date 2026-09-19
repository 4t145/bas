use basr::model::Message;

use super::*;

impl Workspace {
    pub(super) fn render_thought(
        &self,
        id: &str,
        message: &Message,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded.contains(id);
        let key = id.to_owned();
        let copy = message.plain_text();
        let group = SharedString::from(format!("thought-{id}"));
        div()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .id(SharedString::from(format!("expand-{id}")))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .child(Icon::new(IconName::Lightbulb).small())
                    .child(caption("思考", cx).flex_1())
                    .child(
                        Button::new(SharedString::from(format!("copy-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip("复制思考内容")
                            .invisible()
                            .group_hover(group, |style| style.visible())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                            }),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .small(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        this.list.remeasure_items(index..index + 1);
                        cx.notify();
                    })),
            )
            .when(expanded, |panel| {
                panel.child(
                    div()
                        .ml_1p5()
                        .pl_3p5()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(message.content.iter().enumerate().map(
                            |(block_index, block)| {
                                self.render_block(format!("{id}-{block_index}"), block, index, cx)
                            },
                        )),
                )
            })
            .into_any_element()
    }
}
