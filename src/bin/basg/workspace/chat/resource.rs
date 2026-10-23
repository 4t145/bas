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
        tr!("resource-open")
    } else {
        tr!("resource-open-unavailable")
    };
    let uri = resource.uri.clone();
    let group = SharedString::from(format!("resource-{id}"));
    let description = el! {
        .div { flex; flex_col; min_w_0; } [
            .div { # format!("resource-name-{id}"); test_support; truncate; } [~$(resource.name)],
            (~uri, cx).caption { # format!("resource-uri-{id}"); test_support; truncate; },
        ]
    };
    el! {
        .div {
            # ~group; test_support; group: ~group;
            rounded_md; border_1; border_color: $cx.theme().border;
            p_3; min_w_0;
        } [
            (format!("resource-row-{id}"), description).ui::ContentRow::new { icon: IconName::File; } [
                (format!("open-resource-{id}"), IconName::ExternalLink, cx).ui::subtle_icon_button {
                    disabled: $target.is_none(); tooltip: hint;
                    accessibility_label: tr!("resource-open-name", name = resource.name.clone());
                    on_click: $move |_, _, cx| { if let Some(target) = &target { target.open(cx); } };
                },
                ($(SharedString::from(id.to_owned())), IconName::Copy, cx).message_actions::action_button {
                    tooltip: tr!("action-copy-resource");
                    accessibility_label: tr!("action-copy-resource");
                    invisible; group_hover: group, { visible; };
                    on_click: move (_, _, cx) => $cx.write_to_clipboard(ClipboardItem::new_string(uri.clone()));
                },
            ],
        ]
    }.into_any_element()
}
