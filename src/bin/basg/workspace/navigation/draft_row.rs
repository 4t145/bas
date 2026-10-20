use bas::model::Timestamp;
use gpui_kit::component::menu::PopupMenuItem;

use super::{groups, thread_row};
use crate::workspace::*;

pub(in crate::workspace) struct Metadata {
    pub title: Option<String>,
    pub updated_at: Timestamp,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            title: None,
            updated_at: Timestamp::now(),
        }
    }
}

impl Workspace {
    pub(in crate::workspace) fn draft_title(&self) -> String {
        self.navigation
            .drafts
            .metadata
            .get(&self.navigation.selected_directory)
            .and_then(|draft| draft.title.clone())
            .unwrap_or_else(|| crate::i18n::tr!("session-new").into())
    }

    pub(in crate::workspace) fn discard_new_draft(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id.is_some() || self.selected.creating || self.composer.paste_pending {
            return;
        }
        let directory = self.navigation.selected_directory.clone();
        if self
            .renaming
            .edit
            .as_ref()
            .is_some_and(|edit| edit.target == rename::Target::Draft(directory.clone()))
        {
            self.renaming.edit = None;
        } else {
            self.save_title(cx);
        }
        self.selected.error = None;
        self.composer.switch_target(Draft::default(), window, cx);
        self.navigation.drafts.remove(&directory);
        let next = self
            .sessions
            .entries
            .iter()
            .filter(|session| !session.archived)
            .min_by_key(|session| session.cwd.as_deref().map(groups::normalize) != directory)
            .map(|session| session.id);
        if let Some(next) = next {
            self.select(next, window, cx);
            self.navigation.drafts.remove(&directory);
        } else {
            self.selected.waiting_for_agent_selection = true;
            self.focus_input(window, cx);
        }
        cx.notify();
    }

    pub(super) fn draft_thread_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.renaming_draft();
        let group = SharedString::from(format!("draft-thread-{index}"));
        let now = Timestamp::now();
        let updated_at = self
            .navigation
            .drafts
            .metadata
            .get(&self.navigation.selected_directory)
            .map_or(now, |draft| draft.updated_at);
        let disabled = self.selected.creating || self.composer.paste_pending;
        let directory = self.navigation.selected_directory.clone();
        let title = if editing {
            self.render_title_editor(cx)
        } else {
            gds_uim::el! {
                #div { min_w_0; truncate; text_size: $crate::ui::SMALL; } [$self.draft_title()]
            }
            .into_any_element()
        };
        let title_row = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("draft-thread-title-{index}"));
                test_support; h: layout::ICON_BUTTON_SIZE.px; flex; items_center; min_w_0;
                @$!editing => {
                    group_hover: $group.clone(), $gds_uim::m! {
                        pr: $px(thread_row::ACTIONS_WIDTH + thread_row::ACTIONS_GAP);
                    };
                };
            } [$title]
        };
        let actions = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("draft-thread-actions-{index}"));
                test_support; absolute; top_0; right_0; flex; invisible;
                @$!editing => { group_hover: $group.clone(), $gds_uim::m! { visible; }; };
            } [
                #layout::icon_button($SharedString::from(format!("rename-draft-{index}")), IconName::Pencil) {
                    accessibility_label: $crate::i18n::tr!("session-rename");
                    hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("session-rename"), shortcuts::RenameThread);
                    disabled: disabled;
                    on_click: $cx.listener(Self::on_draft_row_rename);
                },
                #layout::icon_button($SharedString::from(format!("discard-draft-{index}")), IconName::Trash) {
                    accessibility_label: $crate::i18n::tr!("draft-discard");
                    tooltip: $crate::i18n::tr!("draft-discard"); disabled: disabled;
                    on_click: $cx.listener(Self::on_draft_row_discard);
                },
            ]
        };
        gds_uim::el! {
            #div {
                id: $group.clone(); test_support; group: group;
                w_full; px_3; py_2; bg: $cx.theme().sidebar_accent;
                hover => { bg: $cx.theme().sidebar_accent; };
                flex; items_start; gap_2;
                on_mouse_down: gpui_kit::MouseButton::Right,
                    $cx.listener(move |this, event, window, cx| {
                        this.on_draft_row_context_menu(directory.clone(), disabled, event, window, cx);
                    });
            } [
                #div { h: layout::ICON_BUTTON_SIZE.px; flex; items_center; flex_shrink_0; } [
                    $self.agent_icon(format!("draft-thread-agent-{index}"), &self.selected.agent, cx),
                ],
                #div { relative; flex_1; min_w_0; flex; flex_col; } [
                    $title_row,
                    #div {
                        id: $SharedString::from(format!("draft-thread-time-{index}"));
                        test_support; h: layout::ICON_BUTTON_SIZE.px; flex; items_center;
                        text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [$crate::ui::time::recent_time(updated_at, now)],
                    $actions,
                ],
            ]
        }.into_any_element()
    }

    fn on_draft_row_rename(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.begin_draft_rename(window, cx);
    }

    fn on_draft_row_discard(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.discard_new_draft(window, cx);
    }

    fn on_draft_row_context_menu(
        &mut self,
        directory: Option<PathBuf>,
        disabled: bool,
        event: &gpui_kit::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        cx.stop_propagation();
        self.open_context_menu(
            event.position,
            2,
            self.draft_menu(directory, disabled, cx),
            window,
            cx,
        );
    }

    fn draft_menu(
        &self,
        directory: Option<PathBuf>,
        disabled: bool,
        cx: &Context<Self>,
    ) -> menus::Builder {
        let owner = cx.entity().downgrade();
        let generation = self.composer.generation;
        std::rc::Rc::new(move |menu, window, cx| {
            let parent = window.window_handle();
            let rename_owner = owner.clone();
            let rename_directory = directory.clone();
            let discard_owner = owner.clone();
            let discard_directory = directory.clone();
            menu.item(
                shortcuts::menu_item(
                    crate::i18n::tr!("session-rename"),
                    Some(Box::new(shortcuts::RenameThread)),
                )
                .icon(IconName::Pencil)
                .disabled(disabled)
                .on_click(move |_, window, cx| {
                    menus::in_parent(parent, window, cx, |window, cx| {
                        let _ = rename_owner.update(cx, |this, cx| {
                            this.on_draft_row_menu_rename(
                                &rename_directory,
                                generation,
                                window,
                                cx,
                            );
                        });
                    });
                }),
            )
            .item(
                PopupMenuItem::new(crate::i18n::tr!("draft-discard"))
                    .icon(crate::ui::delete_icon(cx))
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = discard_owner.update(cx, |this, cx| {
                                this.on_draft_row_menu_discard(
                                    &discard_directory,
                                    generation,
                                    window,
                                    cx,
                                );
                            });
                        });
                    }),
            )
        })
    }

    fn on_draft_row_menu_rename(
        &mut self,
        directory: &Option<PathBuf>,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id.is_none()
            && &self.navigation.selected_directory == directory
            && self.composer.generation == generation
        {
            self.begin_draft_rename(window, cx);
        }
    }

    fn on_draft_row_menu_discard(
        &mut self,
        directory: &Option<PathBuf>,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if &self.navigation.selected_directory == directory
            && self.composer.generation == generation
        {
            self.discard_new_draft(window, cx);
        }
    }
}
