use super::*;
use gpui_kit::component::{
    button::ButtonVariants,
    menu::{DropdownMenu, PopupMenuItem},
};

impl Viewer {
    pub(super) fn level_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.level;
        let owner = cx.entity().downgrade();
        crate::ui::action_button("logs-level")
            .ghost()
            .w(gpui_kit::rems(7.5))
            .flex_shrink_0()
            .label(selected.map_or(crate::i18n::tr!("logs-all"), Level::as_str))
            .icon(IconName::ChevronDown)
            .accessibility_label(crate::i18n::tr!("logs-level"))
            .dropdown_menu(move |menu, _, _| {
                let mut menu = crate::ui::menu(menu);
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
                            level.map_or(crate::i18n::tr!("logs-all"), Level::as_str),
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
            })
            .into_any_element()
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
            .unwrap_or_else(|| crate::i18n::tr!("logs-all-targets").into());
        crate::ui::action_button("logs-target")
            .ghost()
            .w(gpui_kit::rems(11.))
            .flex_shrink_0()
            .tooltip(label.clone())
            .accessibility_label(crate::i18n::tr!("logs-target"))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = crate::ui::menu(menu);
                for target in std::iter::once(None).chain(targets.iter().cloned().map(Some)) {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            target
                                .clone()
                                .unwrap_or_else(|| crate::i18n::tr!("logs-all-targets").into()),
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
            })
            .into_any_element()
    }
}
