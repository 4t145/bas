//! Open supported resource references without interpreting Agent URIs as commands.

use super::super::*;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(super) fn render(
    id: &str,
    resource: &bas::acp::ResourceLink,
    links: &links::Context,
    cx: &App,
) -> AnyElement {
    let target = links.target(&resource.uri).ok();
    let hint = if target.is_some() {
        crate::i18n::tr!("resource-open")
    } else {
        crate::i18n::tr!("resource-open-unavailable")
    };
    let uri = resource.uri.clone();
    let group = SharedString::from(format!("resource-{id}"));
    let description = gds_uim::el! {
        #div { flex; flex_col; min_w_0; } [
            #div { id: $format!("resource-name-{id}"); test_support; truncate; } [$resource.name.clone()],
            $caption(uri.clone(), cx) { id: $format!("resource-uri-{id}"); test_support; truncate; },
        ]
    };
    gds_uim::el! {
        #div {
            id: $group.clone(); test_support; group: $group.clone();
            rounded_md; border_1; border_color: $cx.theme().border;
            p_3; min_w_0;
        } [
            #crate::ui::ContentRow::new($format!("resource-row-{id}"), $description) { icon: IconName::File; } [
            #crate::ui::subtle_icon_button($format!("open-resource-{id}"), IconName::ExternalLink, cx) {
                disabled: $target.is_none(); tooltip: hint;
                accessibility_label: $crate::i18n::tr!("resource-open-name", name = resource.name.clone());
                on_click: $move |_, _, cx| { if let Some(target) = &target { target.open(cx); } };
            },
            $message_actions::action_button(SharedString::from(id.to_owned()), IconName::Copy, cx) {
                tooltip: $crate::i18n::tr!("action-copy-resource");
                accessibility_label: $crate::i18n::tr!("action-copy-resource");
                invisible; group_hover: group, $gds_uim::m! { visible; };
                on_click: $move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(uri.clone()));
            },
            ],
        ]
    }.into_any_element()
}
