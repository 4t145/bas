use crate::workspace::*;
use bas::model::SessionSnapshot;
use gpui_kit::component::menu::PopupMenuItem;

impl Workspace {
    pub(in crate::workspace) fn supports_directories(&self) -> bool {
        self.current()
            .and_then(|meta| meta.initialization.as_ref())
            .is_some_and(|init| {
                let caps = &init.agent_capabilities;
                caps.session_capabilities.additional_directories.is_some()
                    && (caps.load_session || caps.session_capabilities.resume.is_some())
            })
    }

    pub(in crate::workspace) fn can_set_directories(&self) -> bool {
        self.supports_directories()
            && self.current().is_some_and(|meta| {
                self.backend.connected
                    && self.sessions.healthy(meta.id)
                    && !meta.archived
                    && meta.status == SessionStatus::Idle
                    && meta.active_turn.is_none()
                    && !self.sending(meta.id)
                    && !self.navigation.picker.pending
                    && !self.sessions.pending.configuring.contains(&meta.id)
                    && !self.sessions.pending.archiving.contains(&meta.id)
                    && !self.sessions.pending.closing.contains(&meta.id)
                    && !self.sessions.pending.restarting.contains(&meta.id)
                    && self.selected.approval.panel.is_empty()
                    && self.selected.elicitations.is_empty()
            })
    }

    pub(in crate::workspace) fn directories_button(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self.current();
        let target = current.map(|meta| (meta.id, self.selected.epoch));
        let roots = current
            .map(|meta| meta.additional_directories.clone())
            .unwrap_or_default();
        let editable = self.can_set_directories();
        let owner = cx.entity().downgrade();
        let parent = self.window.handle;
        let hint = if editable {
            crate::i18n::tr!("session-directory-add")
        } else {
            crate::i18n::tr!("session-directories-unavailable")
        };
        let button = crate::ui::icon_action("session-directories", IconName::FolderPlus, hint)
            .disabled(target.is_none())
            .loading(
                current.is_some_and(|meta| self.sessions.pending.configuring.contains(&meta.id)),
            );
        self.menu_button(
            button,
            roots.len() + 1,
            target.is_none(),
            move |mut menu, _, _| {
                let add_owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(crate::i18n::tr!("session-directory-add"))
                        .icon(IconName::FolderPlus)
                        .disabled(!editable)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                if let Some((id, epoch)) = target {
                                    let _ = add_owner.update(cx, |this, cx| {
                                        this.choose_additional_directory(id, epoch, window, cx)
                                    });
                                }
                            });
                        }),
                );
                for path in &roots {
                    let owner = owner.clone();
                    let path = path.clone();
                    menu = menu.item(
                        PopupMenuItem::new(crate::i18n::tr!(
                            "session-directory-remove",
                            path = path.display().to_string()
                        ))
                        .icon(IconName::X)
                        .disabled(!editable)
                        .on_click(move |_, _, cx| {
                            if let Some((id, epoch)) = target {
                                let _ = owner.update(cx, |this, cx| {
                                    if this.selected.id == Some(id)
                                        && this.selected.epoch == epoch
                                        && this.can_set_directories()
                                    {
                                        let roots = this
                                            .current()
                                            .expect("selected session")
                                            .additional_directories
                                            .iter()
                                            .filter(|root| **root != path)
                                            .cloned()
                                            .collect();
                                        this.set_directories(id, roots, cx);
                                    }
                                });
                            }
                        }),
                    );
                }
                menu
            },
            cx,
        )
    }

    pub(in crate::workspace) fn choose_additional_directory(
        &mut self,
        id: SessionId,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(id)
            || self.selected.epoch != epoch
            || !self.can_set_directories()
        {
            return;
        }
        self.choose_local_workspace(window, cx);
        self.navigation.picker.session = Some((id, epoch));
    }

    pub(in crate::workspace) fn attach_directories(
        &mut self,
        id: SessionId,
        epoch: u64,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(id)
            || self.selected.epoch != epoch
            || !self.can_set_directories()
        {
            return;
        }
        let meta = self.current().expect("selected session");
        let mut roots = meta.additional_directories.clone();
        for path in paths {
            if meta.cwd.as_ref() != Some(&path) && !roots.contains(&path) {
                roots.push(path);
            }
        }
        if roots != meta.additional_directories {
            self.set_directories(id, roots, cx);
        }
    }

    fn set_directories(
        &mut self,
        session: SessionId,
        directories: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.pending.configuring.insert(session);
        self.command(
            Command::SetDirectories {
                session,
                directories,
            },
            cx,
        );
        cx.notify();
    }

    pub(in crate::workspace) fn directories_updated(
        &mut self,
        session: SessionId,
        result: Result<SessionSnapshot, connection::Failure>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.pending.configuring.remove(&session);
        match result {
            Ok(snapshot) => {
                self.sessions.runtimes.observe(&snapshot, false);
                if let Some(meta) = self
                    .sessions
                    .entries
                    .iter_mut()
                    .find(|meta| meta.id == session)
                {
                    crate::conversation::merge_meta(meta, &snapshot.meta);
                }
                self.selected.timeline.merge_meta(&snapshot.meta);
            }
            Err(error) => self.session_operation_error(
                session,
                crate::i18n::tr!("session-directories-title"),
                error.presentation(),
                cx,
            ),
        }
        cx.notify();
    }
}
