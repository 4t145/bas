use gpui_kit::component::text::{SelectionFormat, TextViewState, TextViewStyle};

use super::*;

pub(super) struct Document {
    pub state: Entity<TextViewState>,
    source: String,
    _subscription: Subscription,
}

impl Workspace {
    pub(super) fn markdown(
        &self,
        id: String,
        text: String,
        index: usize,
        cx: &mut Context<Self>,
    ) -> TextView {
        let mut documents = self.markdown.borrow_mut();
        let document = documents.entry(id).or_insert_with(|| {
            let state = cx.new(|cx| TextViewState::markdown("", cx));
            // Parsing can finish after the virtual list measured the row.
            let subscription = cx.observe(&state, move |this, _, cx| {
                if this
                    .conversation
                    .as_ref()
                    .is_some_and(|c| index < c.items.len())
                {
                    this.list.remeasure_items(index..index + 1);
                }
                cx.notify();
            });
            Document {
                state,
                source: String::new(),
                _subscription: subscription,
            }
        });
        if document.source != text {
            document.state.update(cx, |state, cx| {
                if let Some(suffix) = text.strip_prefix(&document.source) {
                    state.push_str(suffix, cx);
                } else {
                    state.set_text(&text, cx);
                }
            });
            document.source = text;
        }
        view(&document.state, cx)
    }
}

pub(super) fn view(state: &Entity<TextViewState>, cx: &App) -> TextView {
    let font_size = theme::content_font_size(cx);
    let mut table = gpui_kit::StyleRefinement::default();
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    // Agent Panel uses compact heading scales rather than document-preview headings.
    // https://github.com/zed-industries/zed/blob/main/crates/markdown/src/markdown.rs
    let mut style = TextViewStyle::default()
        .paragraph_gap(gpui_kit::rems(0.5))
        .heading_font_size(|level, size| {
            size * match level {
                1 => 1.15,
                2 => 1.1,
                3 => 1.05,
                4 => 1.,
                5 => 0.95,
                _ => 0.875,
            }
        })
        .code_block(
            gpui_kit::StyleRefinement::default()
                .p_2()
                .border_1()
                .rounded_md()
                .border_color(cx.theme().border)
                .bg(cx.theme().foreground.opacity(0.025))
                .text_size(font_size)
                .line_height(gpui_kit::relative(theme::CONTENT_LINE_HEIGHT)),
        )
        .table(table)
        .table_head(gpui_kit::StyleRefinement::default().font_weight(FontWeight::SEMIBOLD))
        .inline_code(gpui_kit::HighlightStyle {
            color: Some(cx.theme().foreground),
            background_color: Some(cx.theme().foreground.opacity(0.08)),
            ..Default::default()
        });
    style.heading_base_font_size = font_size;
    TextView::new(state)
        .style(style)
        .font_family(cx.theme().font_family.clone())
        .text_size(font_size)
        .line_height(gpui_kit::relative(theme::CONTENT_LINE_HEIGHT))
        .selectable(true)
        .scrollable(false)
        .selection_format(SelectionFormat::Plain)
        .w_full()
        .on_link_click(|url, _, _, cx| {
            if url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("mailto:")
            {
                cx.open_url(url);
            }
        })
}
