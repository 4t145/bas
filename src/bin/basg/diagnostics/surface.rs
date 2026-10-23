use gds_uim::el;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, IntoElement, Render, Window,
    component::{ActiveTheme, Root},
    div,
    prelude::*,
    px,
};

pub(crate) struct Surface {
    view: AnyView,
}

pub(crate) fn wrap(view: impl Into<AnyView>, cx: &mut App) -> Entity<Surface> {
    let view = view.into();
    cx.new(|_| Surface { view })
}

/// Preserve typed content access through the application-wide notification layer.
pub(crate) fn content(root: &Root, cx: &App) -> AnyView {
    match root.view().clone().downcast::<Surface>() {
        Ok(surface) => surface.read(cx).view.clone(),
        Err(view) => view,
    }
}

impl Render for Surface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notifications = if super::compact_window(window) {
            // Compact windows have no title bar. Cancel the library's title-bar
            // inset without changing notification placement in other windows.
            window.root::<Root>().flatten().map(|root| {
                el! {
                    .div {
                        absolute; inset_0; top: $px(4.) - cx.theme().notification.margins.top;
                    } [~$(root.read(cx).notification)] { into_any_element; }
                }
            })
        } else {
            Root::render_notification_layer(window, cx).map(IntoElement::into_any_element)
        };
        el! {
            .div { relative; size_full; } [~$(self.view), ..notifications]
        }
    }
}
