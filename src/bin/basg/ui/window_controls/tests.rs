use super::*;
use gpui_kit::{Context, Render, TestAppContext, size, test::TestWindowExt};

struct Harness {
    secondary: bool,
    closes: Rc<Cell<usize>>,
    maximizes: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let closes = self.closes.clone();
        let maximizes = self.maximizes.clone();
        if self.secondary {
            TitleBar::new()
                .child("Ordinary window title")
                .on_close_window(move |_, _, _| closes.set(closes.get() + 1))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .h(px(HEIGHT))
                .child(
                    WindowControls::new()
                        .on_maximize(move |_, _, _| maximizes.set(maximizes.get() + 1))
                        .on_close_window(move |_, _, _| closes.set(closes.get() + 1)),
                )
                .into_any_element()
        }
    }
}

#[gpui_kit::test]
fn main_and_secondary_render_identical_control_and_icon_bounds(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let mut measurements = Vec::new();
    for secondary in [false, true] {
        let closes = Rc::new(Cell::new(0));
        let maximizes = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let closes = closes.clone();
            let maximizes = maximizes.clone();
            move |_, _| Harness {
                secondary,
                closes,
                maximizes,
            }
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            if cfg!(target_os = "macos") || cfg!(target_family = "wasm") {
                assert!(window.try_find("window-close").is_none());
                return;
            }
            let mut sizes = Vec::new();
            for id in ["window-minimize", "window-maximize", "window-close"] {
                if window.try_find(id).is_some() {
                    let button = window.find(id).bounds().size;
                    let icon = window
                        .find(gpui_kit::SharedString::from(format!("{id}-icon")))
                        .bounds()
                        .size;
                    assert_eq!(
                        button,
                        size(
                            px(super::super::COMPACT_CONTROL),
                            px(super::super::COMPACT_CONTROL)
                        )
                    );
                    assert_eq!(
                        icon,
                        size(px(super::super::SMALL_ICON), px(super::super::SMALL_ICON))
                    );
                    sizes.push((button, icon));
                }
            }
            measurements.push(sizes);
            window.click("window-close", cx);
            assert_eq!(closes.get(), 1);
            if !secondary && window.try_find("window-maximize").is_some() {
                window.click("window-maximize", cx);
                assert_eq!(maximizes.get(), 1);
            }
        });
    }
    if !measurements.is_empty() {
        assert_eq!(measurements.len(), 2);
        assert_eq!(measurements[0], measurements[1]);
    }
}
