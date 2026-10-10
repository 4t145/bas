use super::*;
use gpui_kit::InputEvent as _;

fn rename_commands(fixture: &mut Fixture) -> Vec<Command> {
    std::iter::from_fn(|| fixture.commands.try_recv().ok())
        .filter(|command| matches!(command, Command::Rename { .. }))
        .collect()
}

#[gpui_kit::test]
fn draft_title_is_saved_to_its_directory_before_switching_and_creation(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.new_thread(Some("/draft/a".into()), window, cx);
            view.begin_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("  draft title  ", window, cx);
            });
            view.new_thread(Some("/draft/b".into()), window, cx);
            assert!(view.title_edit.is_none());
            assert_eq!(view.draft_title(), "新 thread");
            assert_eq!(
                view.new_draft_metadata[&Some("/draft/a".into())]
                    .title
                    .as_deref(),
                Some("draft title")
            );
            view.new_thread(Some("/draft/a".into()), window, cx);
            view.begin_draft_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("created title", window, cx);
            });
            view.waiting_for_agent_selection = false;
            view.prepare_session(cx);
            assert!(view.title_edit.is_none());
        });
        let creates = std::iter::from_fn(|| fixture.commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::Create(create) => Some(create),
                Command::Rename { .. } => panic!("uncreated drafts must remain local"),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(creates.len(), 1);
        assert_eq!(creates[0].title, "created title");
        assert_eq!(creates[0].cwd, "/draft/a");
    })
    .expect("draft titles remain local until initialization");
}

#[gpui_kit::test]
fn title_edits_in_place_without_buttons_and_escape_cancels(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    rename_commands(&mut fixture);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window.find("thread-header").bounds();
        let slot = window.find("thread-title-slot").bounds();
        assert_eq!(
            window.find("workspace-picker-label").label(),
            window.find("workspace-picker").label()
        );
        window.click("rename-session", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).renaming_header());
        assert_eq!(window.find("thread-header").bounds(), header);
        assert_eq!(window.find("thread-title-slot").bounds(), slot);
        let input = window.find("rename-title-input").bounds();
        assert_eq!(
            input.size.height,
            window.find("sidebar-toggle").bounds().size.height
        );
        assert_eq!(input.center().y, slot.center().y);
        assert_eq!(input.size.width, slot.size.width);
        assert!(window.try_find("save-title").is_none());
        assert!(window.try_find("cancel-rename").is_none());
        fixture.view.update(cx, |view, cx| {
            view.rename
                .update(cx, |input, cx| input.set_value("cancel this", window, cx));
        });
        window.press("escape", cx);
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(view.title_edit.is_none());
        assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
        assert!(window.try_find("rename-title-input").is_none());
    })
    .expect("header editing retains layout and supports Escape");
    assert!(rename_commands(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn blur_saves_once_and_keeps_the_new_focus(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    rename_commands(&mut fixture);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture.view.update(cx, |view, cx| {
            view.begin_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("  saved on blur  ", window, cx);
            });
        });
        window.render_frame(cx);
    })
    .expect("show the title editor before changing focus");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.is_window_active());
        assert!(
            fixture
                .view
                .read(cx)
                .rename
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.render_frame(cx);
    })
    .expect("focus leaves the title editor");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        fixture.view.update(cx, |view, cx| {
            assert!(view.title_edit.is_none());
            assert!(view.pending_renames.contains_key(&meta.id));
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            view.save_title(cx);
        });
    })
    .expect("blur ends editing without taking focus back");
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }] if *session == meta.id && title == "saved on blur"
    ));
}

#[gpui_kit::test]
fn only_left_clicking_outside_the_editor_blurs_and_saves(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    rename_commands(&mut fixture);
    let position = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.activate_window();
            fixture.view.update(cx, |view, cx| {
                view.begin_rename(window, cx);
                view.rename.update(cx, |input, cx| {
                    input.set_value("saved from blank space", window, cx);
                });
            });
            window.render_frame(cx);
            let header = window.find("thread-header").bounds();
            let input = window.find("rename-title-input").bounds();
            let position = gpui_kit::point(input.left() + px(5.), header.top() + px(1.));
            assert!(position.y < input.top());
            position
        })
        .expect("nonfocusable header space outside the title input");
    cx.run_until_parked();
    for button in [gpui_kit::MouseButton::Right, gpui_kit::MouseButton::Left] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.is_window_active());
            window.simulate_mouse_move(position, cx);
            window.dispatch_event(
                gpui_kit::MouseDownEvent {
                    button,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::MouseUpEvent {
                    button,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        })
        .expect("physical click outside the editor");
        cx.run_until_parked();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            let view = fixture.view.read(cx);
            if button == gpui_kit::MouseButton::Right {
                assert!(view.renaming_header());
                assert!(view.rename.read(cx).focus_handle(cx).is_focused(window));
            } else {
                assert!(view.title_edit.is_none());
                assert!(!view.composer.read(cx).focus_handle(cx).is_focused(window));
            }
        })
        .expect(
            "right click retains the editor and left click saves without focusing the composer",
        );
    }
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }]
            if *session == meta.id && title == "saved from blank space"
    ));
}

#[gpui_kit::test]
fn empty_and_unchanged_titles_end_editing_without_requests(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.title = Some("existing title".into());
    attach(&fixture, meta.clone(), cx);
    rename_commands(&mut fixture);
    for text in ["  existing title  ", "  "] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.begin_rename(window, cx);
                view.rename
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            });
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .expect("Enter ends a no-op title edit");
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(fixture.view.read(cx).title_edit.is_none());
            assert!(fixture.view.read(cx).pending_renames.is_empty());
            assert_eq!(
                fixture.view.read(cx).current().expect("session").title,
                meta.title
            );
        });
        assert!(rename_commands(&mut fixture).is_empty());
    }
}

#[gpui_kit::test]
fn pending_rename_is_deduplicated_and_failed_attempt_can_be_retried(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    rename_commands(&mut fixture);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.begin_rename(window, cx);
            view.rename
                .update(cx, |input, cx| input.set_value("  changed  ", window, cx));
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .expect("Enter submits a changed title");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(view.title_edit.is_none());
            assert!(view.pending_renames.contains_key(&meta.id));
            view.save_title(cx);
            view.begin_sidebar_rename(meta.id, window, cx);
            assert!(view.title_edit.is_none());
        });
        window.press("enter", cx);
    })
    .expect("the pending target cannot send another rename");
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }] if *session == meta.id && title == "changed"
    ));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Renamed {
                    session: meta.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert!(view.title_edit.is_none());
            assert!(view.pending_renames.is_empty());
            assert_eq!(view.rename_retries[&meta.id], "  changed  ");
            assert!(
                view.error
                    .as_ref()
                    .is_some_and(|error| error.contains("offline"))
            );
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(view.current().expect("session").title, meta.title);
            view.begin_rename(window, cx);
            assert_eq!(view.rename.read(cx).value(), "  changed  ");
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .expect("the failed title remains available for retry without taking focus");
    cx.run_until_parked();
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }] if *session == meta.id && title == "changed"
    ));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Renamed {
                    session: meta.id,
                    result: Ok(()),
                },
                window,
                cx,
            );
            assert!(view.title_edit.is_none());
            assert!(view.rename_retries.is_empty());
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(view.current().expect("session").title, meta.title);
        });
    })
    .expect("success waits for authoritative metadata and keeps focus");
}

#[gpui_kit::test]
fn switching_target_commits_old_input_and_late_blur_and_receipt_preserve_new_editor(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    rename_commands(&mut fixture);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.push(second.clone());
            view.begin_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("first title", window, cx);
            });
            let old_input = view.rename.clone();
            view.select(second.id, window, cx);
            view.begin_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("second attempt", window, cx);
            });
            old_input.update(cx, |_, cx| cx.emit(InputEvent::Blur));
        });
        window.render_frame(cx);
    })
    .expect("selection commits the immutable old target before replacement");
    cx.run_until_parked();
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }] if *session == first.id && title == "first title"
    ));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.active, Some(second.id));
            assert!(view.renaming_header());
            assert_eq!(view.rename.read(cx).value(), "second attempt");
            assert!(view.rename.read(cx).focus_handle(cx).is_focused(window));
            view.update(
                Update::Renamed {
                    session: first.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert!(view.renaming_header());
            assert_eq!(view.rename.read(cx).value(), "second attempt");
            assert!(view.rename.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(view.rename_retries[&first.id], "first title");
            assert!(
                view.error
                    .as_ref()
                    .is_some_and(|error| error.contains("offline"))
            );
            view.save_title(cx);
            view.begin_sidebar_rename(first.id, window, cx);
            assert!(view.renaming_thread(first.id));
            assert_eq!(view.active, Some(second.id));
            assert_eq!(view.rename.read(cx).value(), "first title");
            view.update(
                Update::Renamed {
                    session: second.id,
                    result: Ok(()),
                },
                window,
                cx,
            );
            assert!(view.renaming_thread(first.id));
            assert!(view.rename.read(cx).focus_handle(cx).is_focused(window));
            view.cancel_rename(window, cx);
        });
    })
    .expect("background failures and successes do not replace or focus another editor");
    assert!(matches!(
        rename_commands(&mut fixture).as_slice(),
        [Command::Rename { session, title }] if *session == second.id && title == "second attempt"
    ));
}
