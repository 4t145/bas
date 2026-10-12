use super::*;
use gpui_kit::{PathPromptOptions, component::menu::PopupMenuItem};

impl Workspace {
    pub(super) fn open_workspace_add_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = window.mouse_position();
        // The previous popup dismisses after its item callback. Open the next menu afterward.
        cx.defer_in(window, move |this, window, cx| {
            let owner = cx.entity().downgrade();
            let parent = this.window;
            let pending = this.workspace_picker_pending;
            let build: menus::Builder = std::rc::Rc::new(move |menu, _, _| {
                let owner = owner.clone();
                menu.item(
                    PopupMenuItem::new(crate::i18n::tr!("workspace-local-folder"))
                        .icon(IconName::Folder)
                        .disabled(pending)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.choose_local_workspace(window, cx);
                                });
                            });
                        }),
                )
            });
            this.open_context_menu(position, 1, build, window, cx);
        });
    }

    fn choose_local_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_picker_pending {
            return;
        }
        self.workspace_picker_pending = true;
        self.workspace_picker_waiting_activation = true;
        cx.defer_in(window, |this, window, cx| {
            this.close_menu(window, cx);
            window.activate_window();
            this.resume_workspace_picker(window, cx);
        });
        cx.notify();
    }

    pub(super) fn resume_workspace_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_picker_waiting_activation || !window.is_window_active() {
            return;
        }
        self.workspace_picker_waiting_activation = false;
        // Wait past the native activation event so Linux has recorded the dialog's parent.
        cx.on_next_frame(window, |this, window, cx| {
            if window.is_window_active() {
                this.prompt_local_workspace(window, cx);
            } else {
                this.workspace_picker_waiting_activation = true;
            }
        });
    }

    fn prompt_local_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::tr!("workspace-choose-folder").into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = async { paths.await? }.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.workspace_picker_pending = false;
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            if this.compact {
                                this.toggle_mode(window, cx);
                            }
                            this.new_thread(Some(groups::normalize(&path)), window, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.operation_error(
                        crate::i18n::tr!("workspace-select-failed"),
                        crate::i18n::tr!("workspace-select-error", error = format!("{:#}", error)),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
