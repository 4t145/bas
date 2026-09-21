use super::*;

impl Workspace {
    pub(super) fn cycle_thread(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sessions = self.visible_sessions();
        let query = self.search.read(cx).value().to_lowercase();
        let threads: Vec<_> = self
            .grouped_threads(&sessions, &query)
            .into_iter()
            .flat_map(|group| group.threads)
            .map(|session| session.id)
            .collect();
        if threads.is_empty() {
            return;
        }
        let current = threads.iter().position(|id| Some(*id) == self.active);
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
