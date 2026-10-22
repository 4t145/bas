//! Complete prompt navigation is a read-only catalog, not the loaded body window.
use bas::model::{Message, Role};
use gpui_kit::{ListOffset, ScrollDelta, ScrollWheelEvent};
use std::collections::BTreeMap;

use super::*;

mod history;
mod state;
mod view;
pub(super) use state::Navigation;

pub(super) const RAIL_WIDTH: f32 = 24.;
const WHEEL_STEP_PIXELS: f32 = 24.;

pub(super) fn message_cursor(message: &Message) -> Option<u64> {
    // Conversation derives presentation IDs from the first original event cursor.
    let id = uuid::Uuid::parse_str(&message.id.to_string()).ok()?;
    u64::try_from(id.as_u128()).ok()
}

impl Workspace {
    fn marker_rows(&self) -> BTreeMap<u64, usize> {
        let mut rows = BTreeMap::new();
        let Some(overview) = &self.selected.timeline.navigation.overview else {
            return rows;
        };
        let Some(conversation) = &self.selected.timeline.conversation else {
            return rows;
        };
        for (index, item) in conversation.items.iter().enumerate() {
            let Item::Message { message, .. } = item else {
                continue;
            };
            if message.role != Role::User {
                continue;
            }
            let Some(cursor) = message_cursor(message) else {
                continue;
            };
            let end = overview
                .entries
                .partition_point(|entry| entry.cursor <= cursor);
            if let Some(entry) = end
                .checked_sub(1)
                .and_then(|index| overview.entries.get(index))
                && let Some(row) = conversation.row_for_item(index)
            {
                rows.entry(entry.cursor).or_insert(row);
            }
        }
        rows
    }

    fn active_marker(&self) -> Option<u64> {
        let navigation = &self.selected.timeline.navigation;
        if let Some(pending) = &navigation.location
            && !pending.append
        {
            return Some(pending.cursor);
        }
        if let Some(selected) = navigation.selected {
            return Some(selected);
        }
        if !navigation.detached && self.selected.timeline.list.is_following_tail() {
            return None;
        }
        let top = self.selected.timeline.list.logical_scroll_top().item_ix;
        self.marker_rows()
            .into_iter()
            .rev()
            .find(|(_, row)| *row <= top)
            .map(|(cursor, _)| cursor)
    }

    pub(super) fn scroll_conversation_to_top(&mut self, cx: &mut Context<Self>) {
        if let Some(cursor) = self
            .selected
            .timeline
            .navigation
            .overview
            .as_ref()
            .and_then(|overview| overview.entries.first())
            .map(|entry| entry.cursor)
        {
            self.navigate_to_marker(cursor, cx);
        } else {
            self.selected.timeline.list.scroll_to(ListOffset::default());
            self.load_older_history(cx);
        }
        cx.notify();
    }

    pub(super) fn page_conversation(&mut self, up: bool, cx: &mut Context<Self>) {
        let list = &self.selected.timeline.list;
        let height = list.viewport_bounds().size.height;
        if height > px(0.) {
            list.scroll_by(if up { -height } else { height });
            cx.notify();
        }
    }

    pub(super) fn jump_conversation_turn(&mut self, previous: bool, cx: &mut Context<Self>) {
        let Some(overview) = &self.selected.timeline.navigation.overview else {
            return;
        };
        let active = self.active_marker();
        let current = active.and_then(|cursor| {
            overview
                .entries
                .iter()
                .position(|entry| entry.cursor == cursor)
        });
        let next = match (current, previous) {
            (Some(index), true) => index.checked_sub(1),
            (Some(index), false) => Some(index + 1),
            (None, true) => overview.entries.len().checked_sub(1),
            (None, false) => Some(overview.entries.len()),
        };
        if let Some(cursor) = next
            .and_then(|index| overview.entries.get(index))
            .map(|entry| entry.cursor)
        {
            self.navigate_to_marker(cursor, cx);
        } else if !previous {
            self.scroll_conversation_to_end(cx);
        }
    }

    pub(super) fn scroll_conversation_to_end(&mut self, cx: &mut Context<Self>) {
        let navigation = &mut self.selected.timeline.navigation;
        navigation.selected = None;
        navigation.location = None;
        navigation.location_error = None;
        if navigation.detached {
            navigation.detached = false;
            navigation.after = None;
            self.selected.timeline.history = None;
            self.selected.timeline.conversation = None;
            self.selected.timeline.restore = None;
            self.selected.timeline.list.reset(0);
            self.selected.reload();
            if let Some(session) = self.selected.id {
                self.command(
                    Command::Watch {
                        session,
                        epoch: self.selected.epoch,
                    },
                    cx,
                );
                self.selected.timeline.navigation.overview_request = None;
                self.request_history_overview(0, cx);
            }
        }
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub(super) fn on_navigation_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y,
        };
        if !event.modifiers.alt {
            self.selected.timeline.navigation.wheel_pixels = 0.;
            self.selected.timeline.navigation.selected = None;
            if delta < 0.
                && self
                    .selected
                    .timeline
                    .list
                    .is_scrolled_to_end()
                    .unwrap_or(false)
            {
                self.load_next_history(cx);
            }
            return;
        }
        cx.stop_propagation();
        if delta == 0. {
            return;
        }
        let step = match event.delta {
            ScrollDelta::Lines(_) => true,
            ScrollDelta::Pixels(_) => {
                let accumulated = &mut self.selected.timeline.navigation.wheel_pixels;
                if accumulated.signum() != delta.signum() {
                    *accumulated = 0.;
                }
                *accumulated += delta;
                if accumulated.abs() >= WHEEL_STEP_PIXELS {
                    *accumulated = 0.;
                    true
                } else {
                    false
                }
            }
        };
        if step {
            self.jump_conversation_turn(delta > 0., cx);
        }
    }

    fn toggle_marker_bookmark(&mut self, session: SessionId, cursor: u64, cx: &mut Context<Self>) {
        if self.selected.id != Some(session) || cursor == 0 {
            return;
        }
        let bookmarks = self.navigation.marker_bookmarks.entry(session).or_default();
        let bookmarked = if bookmarks.remove(&cursor) {
            false
        } else {
            bookmarks.insert(cursor);
            true
        };
        let restore = &mut self.navigation.restore;
        if let Some(store) = &mut restore.store {
            store.queue_marker_bookmark(
                &mut restore.pending_save,
                cx.background_executor(),
                session,
                cursor,
                bookmarked,
            );
        }
        cx.notify();
    }
}
