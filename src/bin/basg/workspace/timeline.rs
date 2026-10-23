//! Paged timeline presentation and a small hot set; durable history lives on disk.

use std::collections::VecDeque;

use crate::connection::PreparedHistory;
use gpui_kit::ListOffset;

use super::*;

const HOT_SESSIONS: usize = 4;
const HOT_BYTES: usize = 32 * 1024 * 1024;
const PREFETCH_ROWS: usize = 3;

#[derive(Clone)]
pub(super) struct Position {
    item: Option<String>,
    offset: Pixels,
    following: bool,
    first: u64,
    expanded: HashSet<String>,
}

#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<(SessionId, state::TimelineState, usize)>,
    positions: HashMap<SessionId, Position>,
    bytes: usize,
}

impl Cache {
    #[cfg(all(test, feature = "gui-tests"))]
    pub(super) fn hot_len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn remove(&mut self, id: SessionId) {
        self.take(id);
        self.positions.remove(&id);
    }

    fn take(&mut self, id: SessionId) -> Option<state::TimelineState> {
        let index = self
            .entries
            .iter()
            .position(|(session, _, _)| *session == id)?;
        let (_, timeline, bytes) = self.entries.remove(index)?;
        self.bytes -= bytes;
        Some(timeline)
    }

    fn insert(&mut self, id: SessionId, mut timeline: state::TimelineState) {
        self.take(id);
        if let Some(position) = timeline.position() {
            self.positions.insert(id, position);
        }
        // Parsed documents and decoded images are regenerated only when visible.
        timeline.clear_caches();
        timeline.bounds.borrow_mut().clear();
        if let Some(conversation) = &mut timeline.conversation {
            conversation.notices.clear();
        }
        timeline.loading_before = None;
        timeline.older_error = None;
        timeline.navigation.park();
        if timeline.history.is_none() {
            return;
        }
        let bytes = timeline.retained_bytes();
        if bytes > HOT_BYTES {
            return;
        }
        while self.entries.len() >= HOT_SESSIONS || self.bytes + bytes > HOT_BYTES {
            let Some((_, _, evicted)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= evicted;
        }
        self.bytes += bytes;
        self.entries.push_back((id, timeline, bytes));
    }
}

impl state::TimelineState {
    pub(super) fn retained_bytes(&self) -> usize {
        self.history_weight
            .bytes()
            .saturating_add(self.presentation_bytes)
            .saturating_add(self.presentation_meta_bytes)
            .saturating_add(self.navigation.retained_bytes())
    }

    /// Catalog and operation responses can replace metadata independently of history.
    pub(super) fn merge_meta(&mut self, meta: &SessionMeta) {
        if let Some(conversation) = &mut self.conversation
            && crate::conversation::merge_meta(&mut conversation.session, meta)
        {
            self.presentation_meta_bytes =
                crate::connection::Prepared::from(&conversation.session).bytes;
        }
    }

    fn row_identities(&self) -> Vec<Vec<String>> {
        self.conversation
            .as_ref()
            .map_or_else(Vec::new, row_identities)
    }

    fn show(&mut self, conversation: Conversation, old: Vec<Vec<String>>) {
        // Splice only the changed identity range. In particular, prepending a
        // page must retain the measured suffix and its within-row scroll anchor.
        // reset() discards that anchor and drops native input until the next paint.
        let new = row_identities(&conversation);
        let expanded_members: HashSet<_> = old
            .iter()
            .filter(|row| {
                row.first().is_some_and(|first| {
                    let key = if row.len() > 1 {
                        format!("activity-{first}")
                    } else {
                        first.clone()
                    };
                    self.expanded.contains(&key)
                })
            })
            .flatten()
            .collect();
        for row in &new {
            if row.len() > 1 && row.iter().any(|id| expanded_members.contains(id)) {
                self.expanded.insert(format!("activity-{}", row[0]));
            }
        }
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if prefix + suffix != old.len() || old.len() != new.len() {
            self.list
                .splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
        }
        // Identical identities can have updated text or tool results on resync.
        // This public API preserves the absolute within-row anchor.
        self.list.remeasure_items(0..new.len());
        // Only the metadata snapshot is measured here, never historical message text.
        self.presentation_meta_bytes =
            crate::connection::Prepared::from(&conversation.session).bytes;
        self.conversation = Some(conversation);
    }

    fn position(&self) -> Option<Position> {
        let conversation = self.conversation.as_ref()?;
        let top = self.list.logical_scroll_top();
        Some(Position {
            item: conversation
                .rows
                .get(top.item_ix)
                .and_then(|row| conversation.items.get(row.items.start))
                .map(Item::id),
            offset: top.offset_in_item,
            following: self.list.is_following_tail(),
            first: self
                .history
                .as_ref()
                .and_then(|history| history.events.first())
                .map_or(0, |event| event.cursor.raw()),
            expanded: self.expanded.clone(),
        })
    }

    fn restore_position(&mut self, position: Position) -> bool {
        self.expanded.extend(position.expanded.iter().cloned());
        if position.following {
            return true;
        }
        self.list.pause_following_tail();
        let row = self.conversation.as_ref().and_then(|conversation| {
            let item = conversation
                .items
                .iter()
                .position(|item| Some(item.id()).as_ref() == position.item.as_ref())?;
            conversation.row_for_item(item)
        });
        if let Some(item_ix) = row {
            let offset = ListOffset {
                item_ix,
                offset_in_item: position.offset,
            };
            let current = self.list.logical_scroll_top();
            if current.item_ix != offset.item_ix || current.offset_in_item != offset.offset_in_item
            {
                self.list.scroll_to(offset);
            }
            true
        } else {
            let can_restore = self.history.as_ref().is_some_and(|history| {
                history.before.is_some_and(|before| before > position.first)
            });
            if can_restore {
                self.restore = Some(position);
            }
            !can_restore
        }
    }
}

impl Workspace {
    pub(super) fn timeline_list(cx: &mut Context<Self>) -> ListState {
        let list = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW));
        let view = cx.entity().downgrade();
        list.set_scroll_handler(move |_, _, cx| {
            // The list is borrowed while delivering its scroll callback.
            let view = view.clone();
            let epoch = view.upgrade().map(|view| view.read(cx).selected.epoch);
            cx.defer(move |cx| {
                let _ = view.update(cx, |this, cx| {
                    if Some(this.selected.epoch) != epoch {
                        return;
                    }
                    // Native input supersedes an evicted-cache restoration.
                    this.selected.timeline.restore = None;
                    let list = &this.selected.timeline.list;
                    if !list.is_following_tail()
                        && list.logical_scroll_top().item_ix < PREFETCH_ROWS
                        && this.selected.timeline.older_error.is_none()
                    {
                        this.load_older_history(cx);
                    }
                    cx.notify();
                });
            });
        });
        list.set_follow_mode(FollowMode::Tail);
        list
    }

    pub(super) fn retain_timeline(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.id else {
            return;
        };
        let replacement = state::TimelineState::new(Self::timeline_list(cx));
        let timeline = std::mem::replace(&mut self.selected.timeline, replacement);
        if !self.sessions.removed.contains(&id) {
            self.sessions.history.insert(id, timeline);
        }
    }

    pub(super) fn restore_timeline(&mut self, id: SessionId) -> Option<u64> {
        if let Some(timeline) = self.sessions.history.take(id) {
            let cursor = timeline.conversation.as_ref().map(|state| state.cursor);
            self.selected.timeline = timeline;
            return cursor;
        }
        self.selected.timeline.restore = self.sessions.history.positions.get(&id).cloned();
        None
    }

    pub(super) fn apply_history_window(
        &mut self,
        prepared: PreparedHistory,
        cached: bool,
        cx: &mut Context<Self>,
    ) {
        let PreparedHistory {
            value: mut history,
            mut weight,
        } = prepared;
        let session = history.session.meta.id;
        if self.selected.id != Some(session) {
            return;
        }
        if self
            .selected
            .timeline
            .history
            .as_ref()
            .is_some_and(|old| old.session.meta.id == session && old.cursor > history.cursor)
        {
            return;
        }
        let started = std::time::Instant::now();
        let incoming_events = history.events.len();
        let incoming_context = history.context.len();
        if let Some(old) = &self.selected.timeline.history
            && old.session.meta.id == session
            && let (Some(first), Some(previous)) = (history.events.first(), old.events.first())
            && first.cursor > previous.cursor
            && first.cursor.raw() <= old.cursor.saturating_add(1)
            && history.cursor >= old.cursor
        {
            let prefix = old
                .events
                .partition_point(|event| event.cursor < first.cursor);
            history
                .events
                .splice(0..0, old.events[..prefix].iter().cloned());
            history.context = old.context.clone();
            history.before = old.before;
            weight.retain_prefix(&self.selected.timeline.history_weight, prefix);
        }
        if self.selected.timeline.loading_before.is_some()
            && self.selected.timeline.loading_before != history.before
        {
            self.selected.timeline.loading_before = None;
            self.selected.timeline.older_error = None;
        }
        let position = self
            .selected
            .timeline
            .restore
            .take()
            .or_else(|| self.selected.timeline.position());
        let old_rows = self.selected.timeline.row_identities();
        if !cached {
            self.sessions.runtimes.observe(&history.session, false);
        }
        let mut meta = history.session.meta.clone();
        if let Some(known) = self.sessions.entries.iter().find(|meta| meta.id == session) {
            crate::conversation::merge_meta(&mut meta, known);
        }
        if let Some(old) = &self.selected.timeline.conversation
            && old.session.id == session
        {
            crate::conversation::merge_meta(&mut meta, &old.session);
        }
        // Failed runtimes can have partial display content outside committed history.
        let mut conversation = if !self.sessions.healthy(session)
            && self
                .selected
                .timeline
                .conversation
                .as_ref()
                .is_some_and(|old| old.session.id == session)
        {
            let cursor = self
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("current conversation was checked")
                .cursor;
            let suffix = history
                .events
                .partition_point(|event| event.cursor.raw() <= cursor);
            if history.cursor > cursor {
                self.selected.timeline.presentation_bytes = self
                    .selected
                    .timeline
                    .presentation_bytes
                    .saturating_add(weight.suffix_bytes(suffix).saturating_mul(2));
            }
            self.selected
                .timeline
                .conversation
                .take()
                .expect("current conversation was checked")
        } else {
            self.selected.timeline.presentation_bytes = weight.bytes().saturating_mul(2);
            Conversation::new(meta.clone())
        };
        conversation.merge_meta(&meta);
        let mut events: Vec<_> = history
            .context
            .iter()
            .chain(&history.events)
            .cloned()
            .collect();
        events.sort_by_key(|event| event.cursor);
        events.dedup_by_key(|event| event.cursor);
        let complete = history.before.is_none()
            && history.events.iter().enumerate().all(|(index, event)| {
                event.session == session && event.cursor.raw() == index as u64 + 1
            });
        if !complete {
            self.sessions.unstarted.remove(&session);
        } else if !history.events.is_empty()
            && history
                .events
                .last()
                .is_some_and(|event| event.cursor >= conversation.session.revision)
            && !self.sessions.prompted.contains(&session)
        {
            self.sessions.unstarted.insert(session);
        }
        if conversation.session.status == SessionStatus::Working
            || conversation.session.active_turn.is_some()
        {
            self.record_user_prompt(session);
        }
        let replay_started = std::time::Instant::now();
        for event in events {
            self.observe_user_prompt(&event);
            if let Some(outbox) = self.sessions.outboxes.get_mut(&session) {
                outbox.observe(&event);
            }
            conversation.apply(event);
        }
        let replay_elapsed = replay_started.elapsed();
        if conversation.foreground_state.is_none() {
            conversation.foreground_state =
                Some(if conversation.session.status == SessionStatus::Working {
                    bas::acp_v2::StateUpdate::Running(bas::acp_v2::RunningStateUpdate::new())
                } else {
                    bas::acp_v2::StateUpdate::Idle(bas::acp_v2::IdleStateUpdate::new())
                });
        }
        conversation.cursor = conversation.cursor.max(history.cursor);
        if let Some(known) = self
            .sessions
            .entries
            .iter_mut()
            .find(|meta| meta.id == session)
        {
            crate::conversation::merge_meta(known, &conversation.session);
        }
        let retained_events = history.events.len();
        let rows = conversation.rows.len();
        let reconcile_started = std::time::Instant::now();
        self.selected.timeline.clear_caches();
        self.selected.timeline.history = Some(history);
        self.selected.timeline.history_weight = weight;
        self.selected.timeline.show(conversation, old_rows);
        if let Some(position) = position
            && !self.selected.timeline.restore_position(position)
        {
            self.load_older_history(cx);
        }
        tracing::debug!(
            incoming_events,
            incoming_context,
            retained_events,
            rows,
            replay_ms = replay_elapsed.as_secs_f64() * 1000.,
            reconcile_ms = reconcile_started.elapsed().as_secs_f64() * 1000.,
            total_ms = started.elapsed().as_secs_f64() * 1000.,
            "applied history window"
        );
    }

    pub(super) fn load_older_history(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected.id else {
            return;
        };
        let timeline = &mut self.selected.timeline;
        if timeline.navigation.location.is_some() {
            return;
        }
        let Some(before) = timeline.history.as_ref().and_then(|history| history.before) else {
            return;
        };
        if timeline.loading_before.is_some() {
            return;
        }
        timeline.loading_before = Some(before);
        timeline.older_error = None;
        self.command(
            Command::OlderHistory {
                session,
                epoch: self.selected.epoch,
                before,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn older_history_loaded(
        &mut self,
        before: u64,
        result: Result<PreparedHistory, Failure>,
        cx: &mut Context<Self>,
    ) {
        let timeline = &mut self.selected.timeline;
        if timeline.loading_before != Some(before)
            || timeline.history.as_ref().and_then(|history| history.before) != Some(before)
        {
            return;
        }
        timeline.loading_before = None;
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                timeline.older_error = Some(error);
                return;
            }
        };
        let Some(history) = timeline.history.as_mut() else {
            return;
        };
        if page.session.meta.id != history.session.meta.id
            || page.events.last().map(|event| event.cursor.raw()) != before.checked_sub(1)
            || page.before.is_some_and(|next| next >= before)
        {
            timeline.older_error = Some(Failure::InvalidPayload(
                "history page does not join the displayed timeline".into(),
            ));
            return;
        }
        let PreparedHistory {
            value: page,
            weight,
        } = page;
        history.before = page.before;
        history.context = page.context;
        history.events.splice(0..0, page.events);
        timeline.history_weight.prepend(weight);
        let history = history.clone();
        // An older response may carry newer metadata, but cannot advance the live
        // cursor or replace events received while that request was in flight.
        let weight = timeline.history_weight.clone();
        self.apply_history_window(
            PreparedHistory {
                value: history,
                weight,
            },
            true,
            cx,
        );
    }

    pub(super) fn render_older_history(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let timeline = &self.selected.timeline;
        let loading = timeline.loading_before.is_some();
        let error = timeline.older_error.as_ref();
        if !loading && error.is_none() {
            return None;
        }
        Some(el! {
            .div {
                # "older-history-status"; test_support;
                absolute; top_0; left_0; right_0; flex; justify_center; px_3;
            } [
                .div {
                    flex; items_center; gap_2; px_3; py_1; max_w_full;
                    rounded_md; bg: $cx.theme().background;
                    text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [
                    @if (loading)
                        ("older-history-loading", IconName::LoaderCircle,
                            $cx.theme().muted_foreground, true).status::activity_icon,
                        tr!("session-history-loading-older"),
                    @end,
                    @if (let Some(error) = error)
                        ("older-history-error", $error.presentation(), $cx.theme().danger)
                            .feedback { compact; },
                        "older-history-retry".action_button {
                            label: tr!("session-history-retry");
                            on_click: $cx.listener(|this, _, _, cx| this.load_older_history(cx));
                        },
                    @end,
                ],
            ]
        }.into_any_element())
    }
}

fn row_identities(conversation: &Conversation) -> Vec<Vec<String>> {
    conversation
        .rows
        .iter()
        .map(|row| {
            conversation.items[row.items.clone()]
                .iter()
                .map(Item::id)
                .collect()
        })
        .collect()
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
