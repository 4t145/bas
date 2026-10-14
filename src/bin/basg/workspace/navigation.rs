//! Directory and thread navigation, sidebar presentation and persisted selection.

use crate::workspace::*;

pub(super) mod draft_row;
mod folders;
pub(super) mod groups;
pub(super) mod restore;
mod sidebar;
pub(super) mod thread_row;

impl Workspace {
    pub(super) fn grouped_threads<'a>(
        &self,
        sessions: &'a [SessionMeta],
        query: &str,
    ) -> Vec<groups::Group<'a>> {
        let mut groups = groups::group_threads(sessions, &self.navigation.directories, query);
        for group in &mut groups {
            group
                .threads
                .sort_by_key(|session| !self.navigation.pinned.contains(&session.id));
        }
        groups
    }

    pub(super) fn toggle_pinned(&mut self, session: SessionId, cx: &mut Context<Self>) {
        let pinned = !self.navigation.pinned.contains(&session);
        if let Some(store) = &mut self.navigation.restore.store {
            if let Err(error) = store.set_pinned(session, pinned) {
                self.session_operation_error(
                    session,
                    crate::i18n::tr!("session-pin-failed"),
                    error.to_string(),
                    cx,
                );
                return;
            }
            self.navigation.pinned = store.pinned();
        } else if pinned {
            self.navigation.pinned.insert(session);
        } else {
            self.navigation.pinned.remove(&session);
        }
        cx.notify();
    }

    pub(super) fn cycle_thread(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sessions = self.visible_sessions();
        let query = self.navigation.search.read(cx).value().to_lowercase();
        let threads: Vec<_> = self
            .grouped_threads(&sessions, &query)
            .into_iter()
            .flat_map(|group| group.threads)
            .map(|session| session.id)
            .collect();
        if threads.is_empty() {
            return;
        }
        let current = threads.iter().position(|id| Some(*id) == self.selected.id);
        let next = match (current, reverse) {
            (Some(index), false) => (index + 1) % threads.len(),
            (Some(index), true) => (index + threads.len() - 1) % threads.len(),
            (None, false) => 0,
            (None, true) => threads.len() - 1,
        };
        self.close_menu(window, cx);
        self.select(threads[next], window, cx);
    }
}
