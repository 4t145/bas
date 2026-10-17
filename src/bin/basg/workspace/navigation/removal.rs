//! Removing a sidebar workspace archives its conversations without deleting data.

use super::groups;
use crate::workspace::*;

pub(in crate::workspace) struct Removal {
    waiting: HashSet<SessionId>,
    failed: bool,
}

impl Workspace {
    pub(in crate::workspace) fn request_remove_workspace(
        &mut self,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_remove_workspace(&directory) {
            return;
        }
        let owner = cx.entity().downgrade();
        let parent = self.window.handle;
        let request = crate::application::Confirmation {
            title: crate::i18n::tr!("workspace-remove").into(),
            message: crate::i18n::tr!(
                "workspace-remove-confirm",
                path = groups::path_label(directory.as_deref())
            ),
            confirm_label: crate::i18n::tr!("workspace-remove").into(),
        };
        if let Err(error) = crate::application::open_confirmation(
            request,
            move |cx| {
                let _ = parent.update(cx, |_, window, cx| {
                    let _ =
                        owner.update(cx, |this, cx| this.remove_workspace(directory, window, cx));
                });
            },
            cx,
        ) {
            self.operation_error(crate::i18n::tr!("workspace-remove"), error.to_string(), cx);
        }
    }

    pub(in crate::workspace) fn can_remove_workspace(&self, directory: &Option<PathBuf>) -> bool {
        self.backend.connected
            && !self.navigation.removing.contains_key(directory)
            && !(self.selected.creating && &self.navigation.selected_directory == directory)
            && !self.composer.paste_pending
            && self
                .sessions
                .entries
                .iter()
                .filter(|session| session.cwd.as_deref().map(groups::normalize) == *directory)
                .all(|session| {
                    let pending = &self.sessions.pending;
                    !pending.archiving.contains(&session.id)
                        && !pending.deleting.contains(&session.id)
                        && !pending.closing.contains(&session.id)
                        && !pending.restarting.contains(&session.id)
                })
    }

    pub(in crate::workspace) fn remove_workspace(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_remove_workspace(&directory) {
            return;
        }
        self.save_title(cx);
        self.save_draft(cx);
        let sessions: HashSet<_> = self
            .sessions
            .entries
            .iter()
            .filter(|session| {
                !session.archived && session.cwd.as_deref().map(groups::normalize) == directory
            })
            .map(|session| session.id)
            .collect();
        self.navigation.removing.insert(
            directory,
            Removal {
                waiting: sessions.clone(),
                failed: false,
            },
        );
        for session in sessions {
            self.set_archived(session, true, cx);
            if !self.sessions.pending.archiving.contains(&session) {
                self.workspace_archive_completed(session, false);
            }
        }
        self.finish_workspace_removals(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn workspace_archive_completed(
        &mut self,
        session: SessionId,
        success: bool,
    ) {
        for removal in self.navigation.removing.values_mut() {
            if removal.waiting.remove(&session) {
                removal.failed |= !success;
            }
        }
    }

    pub(in crate::workspace) fn finish_workspace_removals(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready: Vec<_> = self
            .navigation
            .removing
            .iter()
            .filter(|(_, removal)| removal.waiting.is_empty())
            .map(|(directory, _)| directory.clone())
            .collect();
        for directory in ready {
            let removal = self
                .navigation
                .removing
                .remove(&directory)
                .expect("completed removal");
            let has_live = self.sessions.entries.iter().any(|session| {
                !session.archived && session.cwd.as_deref().map(groups::normalize) == directory
            });
            if removal.failed || has_live {
                self.operation_error(
                    crate::i18n::tr!("workspace-remove"),
                    crate::i18n::tr!("workspace-remove-incomplete").into(),
                    cx,
                );
                continue;
            }
            if !self.set_workspace_hidden(directory.clone(), true, cx) {
                continue;
            }
            self.navigation
                .directories
                .retain(|path| Some(path) != directory.as_ref());
            self.navigation.collapsed.remove(&directory);
            if self.navigation.selected_directory == directory {
                let next = self
                    .sessions
                    .entries
                    .iter()
                    .find(|session| !session.archived)
                    .map(|session| session.id);
                if let Some(next) = next {
                    self.select(next, window, cx);
                } else {
                    self.detach_thread(None, window, cx);
                    self.composer.switch_target(Draft::default(), window, cx);
                    self.focus_input(window, cx);
                }
            }
        }
    }

    pub(in crate::workspace) fn set_workspace_hidden(
        &mut self,
        directory: Option<PathBuf>,
        hidden: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(store) = &mut self.navigation.restore.store
            && let Err(error) = store.set_workspace_hidden(directory.clone(), hidden)
        {
            self.operation_error(
                crate::i18n::tr!("workspace-update-failed"),
                error.to_string(),
                cx,
            );
            return false;
        }
        if hidden {
            self.navigation.hidden.insert(directory);
        } else {
            self.navigation.hidden.remove(&directory);
        }
        true
    }
}
