use gpui_kit::base::{TextViewState, TextViewStyle, text::SelectionFormat};

use super::*;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

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
                    .is_some_and(|c| index < c.rows.len())
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
    TextView::new(state)
        .style(markdown_style(cx))
        .font_family(cx.theme().font_family.clone())
        .text_size(font_size)
        .line_height(gpui_kit::relative(theme::content_line_height(cx)))
        .selectable(true)
        .scrollable(false)
        .selection_format(SelectionFormat::Plain)
        .w_full()
        .table_actions(|table, _, _| {
            let source = table.markdown.clone();
            div().flex().justify_end().child(
                Button::new("copy-markdown-table")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Copy)
                    .tooltip("复制 Markdown 表格")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(source.clone()));
                    }),
            )
        })
        .on_link_click(|url, _, _, cx| {
            if url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("mailto:")
            {
                cx.open_url(url);
            }
        })
}

fn markdown_style(cx: &App) -> TextViewStyle {
    let font_size = theme::content_font_size(cx);
    let mut table = gpui_kit::StyleRefinement::default();
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    let radius = cx.theme().semantic_tokens().radius.md;
    table.corner_radii.top_left = Some(radius.into());
    table.corner_radii.top_right = Some(radius.into());
    table.corner_radii.bottom_left = Some(radius.into());
    table.corner_radii.bottom_right = Some(radius.into());
    // Preserve the component theme's semantic rich-text colors while using
    // Base directly. Syntax highlighting still comes from TextViewDefaults.
    TextViewStyle::default()
        .with_foreground(cx.theme().foreground)
        .with_muted_foreground(cx.theme().muted_foreground)
        .with_link(cx.theme().link)
        .with_selection(cx.theme().selection)
        .with_code_background(cx.theme().muted)
        .with_border(cx.theme().border)
        .with_dark(cx.theme().is_dark())
        .with_heading_base_font_size(font_size)
        .with_paragraph_gap(gpui_kit::rems(0.5))
        .with_heading_font_size(|level, size| {
            size * match level {
                1 => 1.15,
                2 => 1.1,
                3 => 1.05,
                4 => 1.,
                5 => 0.95,
                _ => 0.875,
            }
        })
        .with_code_block(
            gpui_kit::StyleRefinement::default()
                .p_2()
                .border_1()
                .rounded_md()
                .border_color(cx.theme().border)
                .bg(cx.theme().foreground.opacity(0.025))
                .text_size(font_size)
                .line_height(gpui_kit::relative(theme::content_line_height(cx))),
        )
        .with_table(table)
        .with_table_head(
            gpui_kit::StyleRefinement::default()
                .bg(cx.theme().table_head)
                .text_color(cx.theme().table_head_foreground)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .with_inline_code(gpui_kit::HighlightStyle {
            color: Some(cx.theme().foreground),
            background_color: Some(cx.theme().foreground.opacity(0.08)),
            ..Default::default()
        })
}
