use super::*;
use gds_uim::{el, m};
use gpui_kit::{AvailableSpace, ContentMask, MouseButton, canvas};

impl Workspace {
    pub(in crate::workspace) fn render_attachment_chips(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = self.composer.input.clone();
        let owner = cx.entity().downgrade();
        let references = mentions(&editor.read(cx).value(), &self.composer.attachments)
            .into_iter()
            .map(|mention| (mention.range, mention.attachment.clone()))
            .collect::<Vec<_>>();
        canvas(
            move |viewport, window, cx| {
                let mut chips = Vec::new();
                window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                    for (range, attachment) in references {
                        let Some(mut bounds) = editor.read(cx).range_to_bounds(&range) else {
                            continue;
                        };
                        if !bounds.intersects(&viewport) || bounds.size.width <= px(0.) {
                            continue;
                        }
                        bounds.origin.y += px(2.);
                        bounds.size.height -= px(4.);
                        let selected = editor.read(cx).selected_range();
                        let selected = selected.start < range.end && selected.end > range.start;
                        let id = attachment.id;
                        let image = matches!(
                            attachment.content.as_ref(),
                            bas::acp::ContentBlock::Image(_)
                        );
                        let hover_owner = owner.clone();
                        let click_owner = owner.clone();
                        let label = attachment.label.clone();
                        let name = attachment.name.clone();
                        let icon = if image { IconName::Image } else { IconName::File };
                        let background = if selected {
                                cx.theme().primary.opacity(0.18)
                            } else {
                                cx.theme().background
                            };
                        let mut chip = el! {
                            #div {
                                id: $SharedString::from(format!("attachment-{id}-{}", range.start));
                                test_support;
                                flex;
                                items_center;
                                gap: 3.0.px;
                                px: 3.0.px;
                                w: $bounds.size.width;
                                h: $bounds.size.height;
                                rounded: 3.0.px;
                                border_1;
                                border_color: $cx.theme().border;
                                bg: background;
                                text_color: $cx.theme().foreground;
                                font_family: $cx.theme().font_family.clone();
                                text_size: $theme::content_font_size(cx);
                                line_height: $gpui_kit::relative(1.);
                                cursor_pointer;
                                on_mouse_down: MouseButton::Left, $move |_, window, cx| {
                                    Self::on_attachment_chip_click(&click_owner, &range, window, cx);
                                };
                                @image => {
                                    on_hover: $move |hovered, window, cx| {
                                        let _ = hover_owner.update(cx, |this, cx| {
                                            this.hover_attachment(id, *hovered, bounds, window, cx)
                                        });
                                    };
                                };
                                @$!image => {
                                    hoverable_tooltip: $move |window, cx| {
                                        crate::ui::tooltip::view(
                                            crate::i18n::tr!("attachment-select", name = name.as_str()),
                                            window,
                                            cx,
                                        )
                                    };
                                };
                            } [
                                Icon(icon) {
                                    size: crate::ui::SMALL_ICON.px;
                                    flex_shrink_0;
                                },
                                #div { min_w_0; truncate; } [$label],
                            ]
                        }.into_any_element();
                        chip.prepaint_as_root(
                            bounds.origin,
                            bounds.size.map(AvailableSpace::Definite),
                            window,
                            cx,
                        );
                        chips.push(chip);
                    }
                });
                chips
            },
            |viewport, mut chips, window, cx| {
                window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                    for chip in &mut chips {
                        chip.paint(window, cx);
                    }
                });
            },
        )
        .map(m! { absolute; top_0; left_0; size_full; })
        .into_any_element()
    }

    fn on_attachment_chip_click(
        owner: &gpui_kit::WeakEntity<Self>,
        range: &std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.prevent_default();
        cx.stop_propagation();
        let _ = owner.update(cx, |this, cx| {
            this.composer.input.update(cx, |editor, cx| {
                editor.set_selected_range(range.clone(), cx);
                editor.focus(window, cx);
            });
            cx.notify();
        });
    }
}
