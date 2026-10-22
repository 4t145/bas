use super::*;
use crate::connection::{Prepared, PreparedLocation};
use bas::service::HistoryOverview;

impl Workspace {
    pub(in crate::workspace) fn request_history_overview(
        &mut self,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.selected.id else {
            return;
        };
        let navigation = &mut self.selected.timeline.navigation;
        navigation.wanted_revision = navigation.wanted_revision.max(revision);
        if navigation.overview_request.is_some()
            || (revision != 0
                && navigation
                    .overview
                    .as_ref()
                    .is_some_and(|overview| overview.cursor >= navigation.wanted_revision))
        {
            return;
        }
        let request = navigation.next_request();
        navigation.overview_request = Some(request);
        navigation.overview_error = None;
        self.command(
            Command::HistoryOverview {
                session,
                epoch: self.selected.epoch,
                request,
            },
            cx,
        );
    }

    pub(in crate::workspace) fn history_overview_loaded(
        &mut self,
        request: u64,
        result: Result<Prepared<HistoryOverview>, Failure>,
        cx: &mut Context<Self>,
    ) {
        let navigation = &mut self.selected.timeline.navigation;
        if navigation.overview_request != Some(request) {
            return;
        }
        navigation.overview_request = None;
        match result {
            Ok(overview) if self.selected.id == Some(overview.session) => {
                if navigation
                    .overview
                    .as_ref()
                    .is_none_or(|old| old.cursor <= overview.cursor)
                {
                    navigation.overview = Some(overview);
                }
                let revision = navigation.wanted_revision;
                if navigation
                    .overview
                    .as_ref()
                    .is_some_and(|overview| overview.cursor < revision)
                {
                    self.request_history_overview(revision, cx);
                }
            }
            Ok(_) => {}
            Err(error) => navigation.overview_error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::workspace) fn navigate_to_marker(&mut self, cursor: u64, cx: &mut Context<Self>) {
        let row = self.marker_rows().get(&cursor).copied();
        self.selected.timeline.navigation.selected = Some(cursor);
        if let Some(item_ix) = row {
            // Cancel a previous unloaded jump even if this target is already visible.
            self.selected.timeline.navigation.location = None;
            self.selected.timeline.navigation.location_error = None;
            self.selected.timeline.list.pause_following_tail();
            self.selected.timeline.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            self.request_history_location(cursor, false, cx);
        }
        cx.notify();
    }

    fn request_history_location(&mut self, cursor: u64, append: bool, cx: &mut Context<Self>) {
        let Some(session) = self.selected.id else {
            return;
        };
        let navigation = &mut self.selected.timeline.navigation;
        let request = navigation.next_request();
        navigation.location = Some(state::LocationRequest {
            request,
            cursor,
            append,
        });
        navigation.location_error = None;
        self.selected.timeline.loading_before = None;
        if !append {
            self.selected.timeline.older_error = None;
        }
        self.command(
            Command::HistoryLocation {
                session,
                epoch: self.selected.epoch,
                request,
                cursor,
            },
            cx,
        );
    }

    pub(in crate::workspace) fn load_next_history(&mut self, cx: &mut Context<Self>) {
        let navigation = &self.selected.timeline.navigation;
        if navigation.location.is_some() {
            return;
        }
        if let Some(cursor) = navigation.after {
            self.request_history_location(cursor, true, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn history_location_loaded(
        &mut self,
        request: u64,
        result: Result<PreparedLocation, Failure>,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .selected
            .timeline
            .navigation
            .location
            .filter(|pending| pending.request == request)
        else {
            return;
        };
        self.selected.timeline.navigation.location = None;
        let location = match result {
            Ok(location) => location,
            Err(error) => {
                self.selected.timeline.navigation.location_error = Some(error);
                cx.notify();
                return;
            }
        };
        if self.selected.id != Some(location.window.session.meta.id) {
            return;
        }
        if pending.append {
            let previous = self
                .selected
                .timeline
                .history
                .as_ref()
                .and_then(|history| history.events.last())
                .map(|event| event.cursor.raw());
            if previous.is_none_or(|last| {
                location
                    .window
                    .events
                    .first()
                    .is_none_or(|event| event.cursor.raw() > last + 1)
                    || location
                        .window
                        .events
                        .last()
                        .is_none_or(|event| event.cursor.raw() <= last)
            }) {
                self.selected.timeline.navigation.location_error = Some(Failure::InvalidPayload(
                    "history location does not join the displayed range".into(),
                ));
                return;
            }
        } else {
            let timeline = &mut self.selected.timeline;
            timeline.history = None;
            timeline.conversation = None;
            timeline.restore = None;
            timeline.older_error = None;
            timeline.history_weight = Default::default();
            timeline.presentation_bytes = 0;
            timeline.presentation_meta_bytes = 0;
            timeline.clear_caches();
            timeline.bounds.borrow_mut().clear();
            timeline.list.reset(0);
            timeline.list.pause_following_tail();
        }
        let frontier = location.window.cursor;
        self.selected.timeline.navigation.after = location.after;
        self.selected.timeline.navigation.detached = location.after.is_some();
        self.apply_history_window(location.window, false, cx);
        if !pending.append {
            let item_ix = self
                .marker_rows()
                .get(&pending.cursor)
                .copied()
                .unwrap_or(0);
            self.selected.timeline.list.pause_following_tail();
            self.selected.timeline.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
        // Resume from this snapshot: updates that arrived during the jump must
        // not leave a gap when this window reaches the live suffix.
        self.selected.reload();
        if let Some(session) = self.selected.id {
            self.command(
                Command::Resume {
                    session,
                    epoch: self.selected.epoch,
                    cursor: frontier,
                },
                cx,
            );
            self.selected.timeline.navigation.overview_request = None;
            self.request_history_overview(frontier, cx);
        }
        cx.notify();
    }
}
