use super::*;
use gpui_kit::{ScrollStrategy, point};

impl Viewer {
    pub(super) fn move_selection(&mut self, forward: bool, page: bool, cx: &mut Context<Self>) {
        let Some(last) = self.visible.len().checked_sub(1) else {
            return;
        };
        let handle = self.scroll.0.borrow().base_handle.clone();
        let height = f32::from(handle.bounds().size.height);
        let offset = handle.offset();
        let top = f32::from(-offset.y).max(0.);
        let first_visible = ((top / ROW_HEIGHT).floor() as usize).min(last);
        let last_visible = (((top + height) / ROW_HEIGHT).ceil() as usize)
            .saturating_sub(1)
            .min(last);
        let current = self
            .visible
            .iter()
            .position(|&index| Some(self.snapshot.entries[index].id) == self.selected);
        let step = if page {
            (height / ROW_HEIGHT).floor().max(1.) as usize
        } else {
            1
        };
        let next = if !page && current.is_none() {
            if forward { first_visible } else { last_visible }
        } else {
            let current = current.unwrap_or(first_visible);
            if forward {
                current.saturating_add(step).min(last)
            } else {
                current.saturating_sub(step)
            }
        };
        self.selected = Some(self.snapshot.entries[self.visible[next]].id);
        if page {
            let distance = step as f32 * ROW_HEIGHT;
            let next_top = if forward {
                top + distance
            } else {
                (top - distance).max(0.)
            };
            handle.set_offset(point(offset.x, px(-next_top)));
        }
        // Deferred reveal uses the final viewport after the detail panel changes height.
        self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }
}
