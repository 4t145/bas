use super::*;
use crate::ui::action_button;
use gpui_kit::{
    component::{
        button::ButtonVariants,
        menu::{DropdownMenu, PopupMenuItem},
    },
    rems,
};

impl Viewer {
    pub(super) fn level_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.level;
        let owner = cx.entity().downgrade();
        el! {
            "logs-level".action_button {
                ghost; w: 7.5.rems; flex_shrink_0;
                label: $selected.map_or(tr!("logs-all"), Level::as_str);
                icon: IconName::ChevronDown;
                accessibility_label: tr!("logs-level");
                dropdown_menu: $move |menu, _, _| {
                    let mut menu = ui::menu(menu);
                    for level in [
                        None,
                        Some(Level::Trace),
                        Some(Level::Debug),
                        Some(Level::Info),
                        Some(Level::Warn),
                        Some(Level::Error),
                    ] {
                        let owner = owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                level.map_or(tr!("logs-all"), Level::as_str),
                            )
                            .checked(selected == level)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.level = level;
                                    this.filter();
                                    cx.notify();
                                });
                            }),
                        );
                    }
                    menu
                };
                into_any_element;
            }
        }
    }

    pub(super) fn target_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.target.clone();
        let mut targets: std::collections::BTreeSet<_> = self
            .snapshot
            .entries
            .iter()
            .map(|entry| entry.target.clone())
            .collect();
        // Keep an active filter discoverable even after its last entry is evicted.
        targets.extend(selected.iter().cloned());
        let owner = cx.entity().downgrade();
        let label = selected
            .clone()
            .unwrap_or_else(|| tr!("logs-all-targets").into());
        el! {
            "logs-target".action_button {
                ghost; w: 11.0.rems; flex_shrink_0; tooltip: ~label;
                accessibility_label: tr!("logs-target");
            } [
                .div { flex_1; min_w_0; truncate; } [label],
            ] {
                icon: IconName::ChevronDown;
                dropdown_menu: $move |menu, _, _| {
                    let mut menu = ui::menu(menu);
                    for target in std::iter::once(None).chain(targets.iter().cloned().map(Some)) {
                        let owner = owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                target
                                    .clone()
                                    .unwrap_or_else(|| tr!("logs-all-targets").into()),
                            )
                            .checked(selected == target)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.target = target.clone();
                                    this.filter();
                                    cx.notify();
                                });
                            }),
                        );
                    }
                    menu
                };
                into_any_element;
            }
        }
    }
}
