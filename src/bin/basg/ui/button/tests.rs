use super::*;
use gpui_kit::assets::IconName;

#[test]
fn deletion_buttons_share_destructive_styling_without_changing_other_actions() {
    let remove = crate::ui::icon_action("remove", IconName::Trash, "Remove");
    assert!(remove.destructive);
    assert!(remove.disabled(true).destructive);
    assert!(!crate::ui::icon_action("edit", IconName::Pencil, "Edit").destructive);
    assert!(!crate::ui::icon_button("archive", IconName::Archive).destructive);
}
