use super::*;
use gpui_kit::InputEvent as _;

const CODEX_ICON: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/codex-acp.svg";
const PI_ICON: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/pi-acp.svg";

fn agent_icon_catalog() -> bas::management::Catalog {
    use bas::management::{Catalog, Entry, Source, Status};

    let entry = |id: &str, icon: &str| Entry {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        version: "1.0.0".into(),
        repository: String::new(),
        license: String::new(),
        license_url: String::new(),
        icon: icon.into(),
        installed_version: None,
        source: Source::Registry,
        status: Status::NotInstalled,
        message: String::new(),
        spec: None,
    };
    let mut pi = entry("pi", "");
    pi.source = Source::Custom;
    pi.status = Status::Installed;
    pi.spec = Some(bas::config::AgentSpec::acp(
        "pi",
        bas::config::agent::Command::new("/home/atlas/.npm-global/bin/pi-acp", []),
    ));
    Catalog {
        entries: vec![entry("codex-acp", CODEX_ICON), entry("pi-acp", PI_ICON), pi],
        warning: None,
        settings_path: None,
        can_install: false,
    }
}

fn icon_fixture(cx: &mut TestAppContext) -> Fixture {
    cx.update(|cx| {
        crate::agents::metadata::for_test(
            "127.0.0.1:7337".parse().expect("test server"),
            &agent_icon_catalog(),
            cx,
        );
    });
    fixture(cx)
}

#[gpui_kit::test]
fn rows_icons_follow_each_session_agent_and_precede_title(cx: &mut TestAppContext) {
    let fixture = icon_fixture(cx);
    let mut pi = session();
    pi.agent = "pi".into();
    let mut codex = session();
    codex.agent = "codex-acp".into();
    let mut unknown = session();
    unknown.agent = "removed-backend".into();
    for meta in [pi.clone(), codex.clone(), unknown.clone()] {
        attach(&fixture, meta, cx);
    }
    cx.update_window(fixture.window.into(), |_, window, cx| {
        for selected in ["pi", "codex-acp", "removed-backend"] {
            fixture.view.update(cx, |view, cx| {
                view.selected.agent = selected.into();
                cx.notify();
            });
            window.render_frame(cx);
            for (session, expected) in [(&pi, PI_ICON), (&codex, CODEX_ICON)] {
                let icon_id = format!("thread-agent-{}", session.id);
                let icon = window.find(icon_id.clone());
                assert_eq!(
                    crate::agents::icon::rendered_source(&icon_id, cx).as_deref(),
                    Some(expected)
                );
                let title = window.find(format!("thread-title-{}", session.id)).bounds();
                let time = window.find(format!("thread-time-{}", session.id)).bounds();
                assert_eq!(icon.bounds().size, size(px(16.), px(16.)));
                assert_eq!(
                    icon.bounds().left(),
                    window.find("workspace-0").bounds().left(),
                    "provider logos align with the workspace collapse indicator"
                );
                assert!(icon.bounds().right() < title.left());
                assert_eq!(icon.bounds().center().y, title.center().y);
                assert_eq!(time.left(), title.left());
                assert!(time.top() >= title.bottom());
            }
            assert!(
                window
                    .find(format!("thread-agent-{}-fallback", unknown.id))
                    .visible(),
                "a missing backend must keep a visible generic icon"
            );
        }
    })
    .expect("independent session icons and stable row layout");
}

#[gpui_kit::test]
fn draft_icon_follows_selected_agent_and_selected_workspace(cx: &mut TestAppContext) {
    let fixture = icon_fixture(cx);
    let mut existing = session();
    existing.agent = "pi".into();
    existing.cwd = Some("/workspace/a".into());
    attach(&fixture, existing.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.new_thread(Some("/workspace/b".into()), window, cx);
        });
        for (agent, directory, expected) in [
            ("pi", "/workspace/b", PI_ICON),
            ("codex-acp", "/workspace/b", CODEX_ICON),
            ("codex-acp", "/workspace/c", CODEX_ICON),
        ] {
            fixture.view.update(cx, |view, cx| {
                view.select_agent(agent.into(), window, cx);
                if view.navigation.selected_directory.as_deref()
                    != Some(std::path::Path::new(directory))
                {
                    view.new_thread(Some(directory.into()), window, cx);
                }
            });
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            let visible = view.visible_sessions();
            let groups = view.grouped_threads(&visible, "");
            let index = groups
                .iter()
                .position(|group| group.directory == view.navigation.selected_directory)
                .expect("selected workspace is in sidebar");
            let icon_id = format!("draft-thread-agent-{index}");
            let icon = window.find(icon_id.clone());
            assert_eq!(
                crate::agents::icon::rendered_source(&icon_id, cx).as_deref(),
                Some(expected)
            );
            let title = window.find(format!("draft-thread-title-{index}")).bounds();
            assert_eq!(icon.bounds().size, size(px(16.), px(16.)));
            assert_eq!(
                icon.bounds().left(),
                window.find(format!("workspace-{index}")).bounds().left(),
                "draft logos use the same left alignment as workspace headers"
            );
            assert!(icon.bounds().right() < title.left());
            assert_eq!(icon.bounds().center().y, title.center().y);
            for other in 0..groups.len() {
                if other != index {
                    assert!(
                        window
                            .try_find(format!("draft-thread-agent-{other}"))
                            .is_none()
                    );
                }
            }
            assert_eq!(
                crate::agents::icon::rendered_source(&format!("thread-agent-{}", existing.id), cx,)
                    .as_deref(),
                Some(PI_ICON),
                "changing the draft agent does not relabel an existing thread"
            );
            assert_eq!(
                crate::agents::icon::rendered_source("agent-picker-icon", cx).as_deref(),
                Some(expected)
            );
        }
    })
    .expect("draft icon follows the agent used to create it");
}

#[gpui_kit::test]
fn rows_receive_icons_when_shared_catalog_arrives_after_sessions(cx: &mut TestAppContext) {
    let empty = bas::management::Catalog {
        entries: Vec::new(),
        warning: None,
        settings_path: None,
        can_install: false,
    };
    let server = "127.0.0.1:7337".parse().expect("test server");
    cx.update(|cx| crate::agents::metadata::for_test(server, &empty, cx));
    let fixture = fixture(cx);
    let mut meta = session();
    meta.agent = "codex-acp".into();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find(format!("thread-agent-{}-fallback", meta.id))
                .visible()
        );
        crate::agents::metadata::publish(server, &agent_icon_catalog(), cx);
    })
    .expect("catalog publishes metadata after session restoration");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            crate::agents::icon::rendered_source(&format!("thread-agent-{}", meta.id), cx)
                .as_deref(),
            Some(CODEX_ICON)
        );
    })
    .expect("sidebar observes shared metadata changes");
}

#[gpui_kit::test]
fn rows_status_changes_preserve_title_time_and_hover_actions(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let target = session();
    let id = target.id;
    attach(&fixture, target, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let title_id = format!("thread-title-{id}");
        let icon_id = format!("thread-agent-{id}");
        let rail_id = format!("thread-rail-{id}");
        let title = window.find(title_id.clone()).bounds();
        let icon = window.find(icon_id.clone()).bounds();
        let rail = window.find(rail_id.clone()).bounds();
        let time_id = format!("thread-time-{id}");
        let time = window.find(time_id.clone()).bounds();
        for (status, unread, dimensions) in [
            (SessionStatus::Starting, true, (12., 12.)),
            (SessionStatus::Working, true, (12., 12.)),
            (SessionStatus::Idle, true, (6., 6.)),
            (SessionStatus::Idle, false, (0., 0.)),
            (SessionStatus::Failed, false, (12., 12.)),
            (SessionStatus::Stopped, false, (6., 6.)),
        ] {
            fixture.view.update(cx, |view, cx| {
                let session = view
                    .sessions
                    .entries
                    .iter_mut()
                    .find(|session| session.id == id)
                    .expect("row");
                session.status = status;
                session.last_reply = Cursor::from_raw(9);
                view.sessions
                    .seen_replies
                    .insert(id, if unread { 8 } else { 9 });
                cx.notify();
            });
            window.hover("thread-search", cx);
            window.render_frame(cx);
            assert_eq!(window.find(rail_id.clone()).bounds(), rail);
            assert_eq!(window.find(title_id.clone()).bounds(), title);
            assert_eq!(window.find(icon_id.clone()).bounds(), icon);
            assert_eq!(window.find(time_id.clone()).bounds(), time);
            let indicator = window.find(format!("thread-indicator-{id}")).bounds();
            assert_eq!(indicator.size, size(px(dimensions.0), px(dimensions.1)));
            if dimensions.0 > 0. {
                assert_eq!(indicator.center(), rail.center());
            }
            window.hover(format!("session-{id}"), cx);
            if dimensions.0 == 0. {
                assert!(window.try_find(format!("thread-status-{id}")).is_none());
            } else {
                let status = window.find(format!("thread-status-{id}"));
                assert!(status.visible());
                assert_eq!(status.role(), Some(gpui_kit::Role::Status));
                assert_eq!(status.focused(), None);
            }
            assert!(window.find(format!("rename-{id}")).visible());
            assert!(window.find(format!("archive-{id}")).visible());
            assert_eq!(
                window.find(format!("archive-{id}")).bounds().right(),
                rail.right()
            );
            assert_eq!(window.find(title_id.clone()).bounds(), title);
            assert_eq!(window.find(icon_id.clone()).bounds(), icon);
            assert_eq!(window.find(time_id.clone()).bounds(), time);
        }
    })
    .expect("status geometry remains stable");
}

#[gpui_kit::test]
fn rows_and_workspace_headers_hover_across_full_width_with_adjacent_actions(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("thread-search", cx);
        let row_id = format!("session-{}", first.id);
        let title_id = format!("thread-title-{}", first.id);
        let status_id = format!("thread-status-{}", first.id);
        let rename_id = format!("rename-{}", first.id);
        let archive_id = format!("archive-{}", first.id);
        let row = window.find(row_id.clone()).bounds();
        let title = window.find(title_id.clone()).bounds();
        let rail = window.find(format!("thread-rail-{}", first.id)).bounds();
        let header = window.find("workspace-header-0").bounds();
        let plus = window.find("new-thread-0").bounds();
        let more = window.find("workspace-menu-0").bounds();
        assert_eq!(row.left(), header.left());
        assert_eq!(row.right(), header.right());
        assert!(row.size.width >= px(SIDEBAR_WIDTH - 1.));
        assert_eq!(plus.right(), more.left());
        assert!(window.try_find(status_id.clone()).is_none());
        assert!(!window.find(rename_id.clone()).visible());
        assert!(!window.find(archive_id.clone()).visible());
        assert!(!window.find("new-thread-0").visible());
        assert!(!window.find("workspace-menu-0").visible());
        for x in [header.left() + px(1.), header.right() - px(1.)] {
            window.simulate_mouse_move(gpui_kit::point(x, header.center().y), cx);
            window.render_frame(cx);
            assert!(window.find("new-thread-0").visible());
            assert!(window.find("workspace-menu-0").visible());
        }
        for x in [row.left() + px(1.), row.right() - px(1.)] {
            window.simulate_mouse_move(gpui_kit::point(x, row.center().y), cx);
            window.render_frame(cx);
            assert!(window.try_find(status_id.clone()).is_none());
            assert!(window.find(rename_id.clone()).visible());
            assert!(window.find(archive_id.clone()).visible());
        }
        assert!(
            window
                .try_find(format!("thread-status-{}", second.id))
                .is_none()
        );
        assert!(!window.find(format!("rename-{}", second.id)).visible());
        assert!(!window.find(format!("archive-{}", second.id)).visible());
        let rename = window.find(rename_id.clone()).bounds();
        let archive = window.find(archive_id.clone()).bounds();
        assert_eq!(rename.right(), archive.left());
        assert_eq!(rename.center().y, archive.center().y);
        assert_eq!(rename.size, archive.size);
        assert_eq!(archive.right(), rail.right());
        assert_eq!(rename.left(), plus.left());
        assert_eq!(archive.right(), more.right());
        assert_eq!(window.find(row_id).bounds(), row);
        assert_eq!(window.find(title_id).bounds(), title);
        window.hover("thread-search", cx);
        assert!(window.try_find(status_id).is_none());
        assert!(!window.find(rename_id).visible());
        assert!(!window.find(archive_id).visible());
        assert!(!window.find("new-thread-0").visible());
        assert!(!window.find("workspace-menu-0").visible());
    })
    .expect("full width hover and grouped actions");
}

#[gpui_kit::test]
fn rows_archive_filter_is_outside_input_on_same_line(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let input = window.find("thread-search").bounds();
        let filter = window.find("archived-list").bounds();
        assert!(filter.left() > input.right());
        assert!((filter.center().y - input.center().y).abs() <= px(0.5));
        window.click("archived-list", cx);
        assert_eq!(
            fixture.view.read(cx).navigation.view,
            workspace_navigation::View::Archived
        );
    })
    .expect("archive list control outside input border");
}

#[gpui_kit::test]
fn rows_archive_hover_action_does_not_select_target(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", target.id), cx);
        window.click(format!("archive-{}", target.id), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Archive { session, archived: true }) if session == target.id
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("archive does not select or send");
}

#[gpui_kit::test]
fn rows_rename_hover_action_opens_the_target_title_editor(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("activate before editing");
    cx.run_until_parked();
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", target.id), cx);
        window.click(format!("rename-{}", target.id), cx);
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(fixture.view.read(cx).renaming_thread(target.id));
        assert!(
            window
                .within(format!("thread-title-{}", target.id))
                .find("rename-title-input")
                .visible()
        );
        assert!(window.try_find("save-title").is_none());
        assert!(window.try_find("cancel-rename").is_none());
        window.click("rename-title-input", cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        fixture.view.update(cx, |view, cx| {
            view.renaming.input.update(cx, |input, cx| {
                input.set_value("renamed sidebar thread", window, cx)
            });
        });
    })
    .expect("rename targets the hovered row");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("composer-editor", cx);
        window.render_frame(cx);
    })
    .expect("leave the sidebar title for the composer");
    cx.run_until_parked();
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Rename {session, title}) if session == target.id && title == "renamed sidebar thread"
    ));
    assert!(fixture.commands.try_recv().is_err());
    cx.update(|cx| {
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(!fixture.view.read(cx).renaming_thread(target.id));
    });
}

#[gpui_kit::test]
fn rows_inline_title_right_click_keeps_edit_and_metadata_click_selects(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", target.id), cx);
        window.click(format!("rename-{}", target.id), cx);
        fixture.view.update(cx, |view, cx| {
            view.renaming.input.update(cx, |input, cx| {
                input.set_value("title from row editor", window, cx)
            });
        });
        window.right_click("rename-title-input", cx);
    })
    .expect("right-click inside the title input");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(active.id));
        assert!(view.renaming_thread(target.id));
        assert!(
            view.window.menu.is_none(),
            "input right-click must not open thread operations"
        );
        assert_eq!(
            view.renaming.input.read(cx).value(),
            "title from row editor"
        );
        assert!(window.try_find("popup-menu").is_none());
        assert!(fixture.commands.try_recv().is_err());
        window.click(format!("thread-time-{}", target.id), cx);
    })
    .expect("right-click preserves the editor and metadata selects the row");
    cx.run_until_parked();
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(target.id));
        assert!(!view.renaming_thread(target.id));
    });
    let mut renames = 0;
    while let Ok(command) = fixture.commands.try_recv() {
        match command {
            Command::Rename { session, title } => {
                assert_eq!(session, target.id);
                assert_eq!(title, "title from row editor");
                renames += 1;
            }
            Command::Watch { session, .. } => assert_eq!(session, target.id),
            _ => panic!("row selection must only rename and watch its thread"),
        }
    }
    assert_eq!(renames, 1);
}

#[gpui_kit::test]
fn rows_unstarted_session_hover_discards_instead_of_archiving(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.unstarted.insert(target.id);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find(format!("archive-{}", target.id)).is_none());
        assert!(!window.find(format!("discard-{}", target.id)).visible());
        window.hover(format!("session-{}", target.id), cx);
        let rename = window.find(format!("rename-{}", target.id));
        let discard = window.find(format!("discard-{}", target.id));
        assert!(rename.visible());
        assert!(discard.visible());
        assert_eq!(rename.bounds().right(), discard.bounds().left());
        window.click(format!("discard-{}", target.id), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(
            fixture
                .view
                .read(cx)
                .sessions
                .pending
                .deleting
                .contains(&target.id)
        );
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Delete(session)) if session == target.id
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("discard sends deletion for an unstarted thread only");
}

#[gpui_kit::test]
fn rows_local_draft_hover_renames_and_discards_without_creating_a_session(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.new_thread(Some("/tmp".into()), window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("draft prompt", window, cx));
        });
        while fixture.commands.try_recv().is_ok() {}
        window.render_frame(cx);
        window.hover("thread-search", cx);
        let row = window.find("draft-thread-0").bounds();
        let title = window.find("draft-thread-title-0").bounds();
        let time = window.find("draft-thread-time-0").bounds();
        assert_eq!(
            row.left(),
            window.find("workspace-header-0").bounds().left()
        );
        assert_eq!(
            row.right(),
            window.find("workspace-header-0").bounds().right()
        );
        assert_eq!(time.left(), title.left());
        assert!(time.top() >= title.bottom());
        assert!(!window.find("rename-draft-0").visible());
        assert!(!window.find("discard-draft-0").visible());
        window.hover("draft-thread-0", cx);
        let rename = window.find("rename-draft-0");
        let discard = window.find("discard-draft-0");
        assert!(rename.visible());
        assert!(discard.visible());
        assert_eq!(rename.bounds().right(), discard.bounds().left());
        window.click("rename-draft-0", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).renaming_draft());
        assert!(
            window
                .within("draft-thread-title-0")
                .find("rename-title-input")
                .visible()
        );
        fixture.view.update(cx, |view, cx| {
            view.renaming
                .input
                .update(cx, |input, cx| input.set_value("local title", window, cx));
        });
    })
    .expect("local draft title edits in its sidebar row");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("composer-editor", cx);
        window.render_frame(cx);
    })
    .expect("leave the local draft editor for the composer");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).draft_title(), "local title");
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(fixture.commands.try_recv().is_err());
        window.hover("draft-thread-0", cx);
        window.click("discard-draft-0", cx);
        let view = fixture.view.read(cx);
        assert!(view.selected.id.is_none());
        assert!(view.composer.input.read(cx).value().is_empty());
        assert!(view.navigation.drafts.contents.is_empty());
        assert!(view.navigation.drafts.metadata.is_empty());
        assert_eq!(view.draft_title(), "新建会话");
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("local draft actions need no daemon session");
}

#[gpui_kit::test]
fn rows_discarding_local_draft_preserves_another_threads_title_edit(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut target = session();
    target.cwd = Some("/tmp".into());
    attach(&fixture, target.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.new_thread(Some("/tmp".into()), window, cx);
            view.backend.connected = true;
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("local draft", window, cx));
        });
        while fixture.commands.try_recv().is_ok() {}
        window.hover(format!("session-{}", target.id), cx);
        window.click(format!("rename-{}", target.id), cx);
        window.click("rename-title-input", cx);
        window.press("ctrl-a", cx);
        window.input("preserved thread title", cx);
        assert!(fixture.view.read(cx).selected.id.is_none());
        assert!(fixture.view.read(cx).renaming_thread(target.id));
        window.hover("draft-thread-0", cx);
        window.click("discard-draft-0", cx);
    })
    .expect("discard an unrelated local draft while editing an inactive thread");
    cx.run_until_parked();
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(target.id));
        assert!(view.renaming.edit.is_none());
        assert!(view.composer.input.read(cx).value().is_empty());
        assert!(view.navigation.drafts.contents.is_empty());
        assert!(view.navigation.drafts.metadata.is_empty());
    });
    let mut renames = 0;
    while let Ok(command) = fixture.commands.try_recv() {
        match command {
            Command::Rename { session, title } => {
                assert_eq!(session, target.id);
                assert_eq!(title, "preserved thread title");
                renames += 1;
            }
            Command::Watch { session, .. } => assert_eq!(session, target.id),
            _ => panic!("discarding a local draft must only save and select the remaining thread"),
        }
    }
    assert_eq!(renames, 1);
}

#[gpui_kit::test]
fn rows_workspace_indicator_tracks_collapse_and_plus_does_not_toggle_it(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    attach(&fixture, target.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("workspace-header-0").expanded(), Some(true));
        window.click("workspace-0", cx);
        assert_eq!(window.find("workspace-header-0").expanded(), Some(false));
        assert!(window.try_find(format!("session-{}", target.id)).is_none());
        window.click("workspace-0", cx);
        assert_eq!(window.find("workspace-header-0").expanded(), Some(true));
        assert!(window.find(format!("session-{}", target.id)).visible());
        window.hover("workspace-header-0", cx);
        window.click("new-thread-0", cx);
        assert_eq!(window.find("workspace-header-0").expanded(), Some(true));
        assert!(fixture.view.read(cx).selected.id.is_none());
        assert!(window.find("draft-thread-0").visible());
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Create(request)) if request.agent == target.agent.to_string()));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("workspace collapse and independent new-thread action");
}

#[gpui_kit::test]
fn collapsed_workspace_rows_touch_and_keep_their_boundary_interactive(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut first = session();
    first.cwd = Some("/workspace/first".into());
    let mut second = session();
    second.cwd = Some("/workspace/second".into());
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    let position = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.navigation.directories.clear();
                view.navigation.collapsed.insert(first.cwd.clone());
                view.navigation.collapsed.insert(second.cwd.clone());
                cx.notify();
            });
            window.render_frame(cx);
            let upper = window.find("workspace-header-0").bounds();
            let lower = window.find("workspace-header-1").bounds();
            assert_eq!(upper.bottom(), lower.top());
            assert_eq!(upper.left(), lower.left());
            assert_eq!(upper.right(), lower.right());
            assert_eq!(window.find("workspace-header-0").expanded(), Some(false));
            assert_eq!(window.find("workspace-header-1").expanded(), Some(false));
            for (index, y) in [(0, upper.bottom() - px(0.5)), (1, lower.top() + px(0.5))] {
                window.simulate_mouse_move(gpui_kit::point(lower.left() + px(1.), y), cx);
                window.render_frame(cx);
                assert!(window.find(format!("workspace-menu-{index}")).visible());
                assert!(window.find(format!("new-thread-{index}")).visible());
                assert!(
                    !window
                        .find(format!("workspace-menu-{}", 1 - index))
                        .visible()
                );
            }
            gpui_kit::point(lower.left() + px(1.), lower.top() + px(0.5))
        })
        .expect("collapsed workspace headers have contiguous hover regions");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    })
    .expect("right click at the shared workspace boundary");
    cx.run_until_parked();
    let menu = cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .window
            .menu
            .as_ref()
            .expect("workspace menu from the boundary")
            .window
            .unwrap_or(fixture.window.into())
    });
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(2usize, cx);
    })
    .expect("copy directory from the clicked workspace");
    assert_eq!(
        cx.read_from_clipboard()
            .expect("copied workspace path")
            .text(),
        Some("/workspace/second".into())
    );
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(second.id));
        assert!(view.navigation.collapsed.contains(&first.cwd));
        assert!(view.navigation.collapsed.contains(&second.cwd));
    });
}

// Context menus are built deferred by the component. Keep opening and confirming
// in separate updates so keyboard dispatch sees the rendered menu focus.
fn open_row_menu(
    fixture: &Fixture,
    id: SessionId,
    cx: &mut TestAppContext,
) -> gpui_kit::AnyWindowHandle {
    open_sidebar_context_menu(fixture, format!("thread-time-{id}"), cx)
}

fn open_sidebar_context_menu(
    fixture: &Fixture,
    target: String,
    cx: &mut TestAppContext,
) -> gpui_kit::AnyWindowHandle {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click(target, cx);
    })
    .expect("open row menu");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
    })
    .expect("render row menu");
    cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .window
            .menu
            .as_ref()
            .and_then(|menu| menu.window)
            .unwrap_or(fixture.window.into())
    })
}

#[gpui_kit::test]
fn rows_stale_draft_context_menu_cannot_discard_a_replacement_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.new_thread(Some("/tmp".into()), window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("old draft", window, cx));
        });
        while fixture.commands.try_recv().is_ok() {}
    })
    .expect("prepare local draft");
    let menu = open_sidebar_context_menu(&fixture, "draft-thread-time-0".into(), cx);
    let content = cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .window
            .menu
            .as_ref()
            .expect("open draft menu")
            .content
            .clone()
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.new_thread(Some("/tmp".into()), window, cx);
            view.composer.input.update(cx, |input, cx| {
                input.set_value("replacement draft", window, cx)
            });
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("replace draft after opening its menu");
    cx.update_window(menu, |_, window, cx| {
        content.focus_handle(cx).focus(window, cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        let view = fixture.view.read(cx);
        assert!(view.selected.id.is_none());
        assert_eq!(
            view.composer.input.read(cx).value().as_str(),
            "replacement draft"
        );
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("stale menu leaves the replacement draft intact");
}

#[gpui_kit::test]
fn rows_stale_thread_menu_cannot_start_editing_an_archived_hidden_row(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut archived = target.clone();
            archived.archived = true;
            archived.revision = Cursor::from_raw(target.revision.raw() + 1);
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![archived.into(), active.clone().into()],
                })),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(window.try_find(format!("session-{}", target.id)).is_none());
        assert!(fixture.view.read(cx).window.menu.is_some());
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
    })
    .expect("catalog hides the target while its existing menu remains open");
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("click rename in the old thread menu");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(active.id));
        assert!(view.renaming.edit.is_none());
        assert!(window.try_find("rename-title-input").is_none());
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("a stale menu creates neither an invisible editor nor a rename request");
}

#[gpui_kit::test]
fn rows_workspace_right_click_and_more_menu_toggle_the_same_group(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    attach(&fixture, target.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_sidebar_context_menu(&fixture, "workspace-header-0".into(), cx);
    cx.update_window(menu, |_, window, cx| {
        assert_eq!(fixture.view.read(cx).selected.id, Some(target.id));
        assert!(!fixture.view.read(cx).navigation.collapsed.contains(&None));
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(fixture.view.read(cx).navigation.collapsed.contains(&None));
        assert_eq!(fixture.view.read(cx).selected.id, Some(target.id));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("workspace context menu collapses its group");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("workspace-header-0", cx);
        window.click("workspace-menu-0", cx);
        assert!(fixture.view.read(cx).navigation.collapsed.contains(&None));
    })
    .expect("more button opens without toggling workspace");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(!fixture.view.read(cx).navigation.collapsed.contains(&None));
        assert_eq!(fixture.view.read(cx).selected.id, Some(target.id));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("more menu expands the same group");
}

#[gpui_kit::test]
fn draft_context_menu_has_only_one_destructive_action(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let draft = session();
    attach(&fixture, draft.clone(), cx);
    cx.update(|cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.unstarted.insert(draft.id);
            cx.notify();
        })
    });
    let menu = open_row_menu(&fixture, draft.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(crate::i18n::tr!("draft-discard")).visible());
        assert!(
            window
                .try_find(crate::i18n::tr!("session-delete"))
                .is_none()
        );
        assert!(
            window
                .try_find(crate::i18n::tr!("session-archive"))
                .is_none()
        );
    })
    .expect("one draft removal entry");
}

#[gpui_kit::test]
fn stopped_thread_menu_starts_its_target_without_switching_the_current_thread(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let mut target = session();
    target.status = SessionStatus::Stopped;
    target.initialization = Some(
        bas::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(bas::acp::AgentCapabilities::new().load_session(true)),
    );
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("启动会话").visible());
        window.click("启动会话", cx);
    })
    .expect("start from the actual context menu");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == target.id)
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.request_restart_agent_session(target.id, cx);
            assert!(
                fixture.commands.try_recv().is_err(),
                "pending restart is deduplicated"
            );
            target.status = SessionStatus::Idle;
            target.revision = Cursor::from_raw(1);
            view.update(
                Update::RestartedSession {
                    session: target.id,
                    result: Ok(target.clone().into()),
                },
                window,
                cx,
            );
            assert!(!view.sessions.pending.restarting.contains(&target.id));
            assert_eq!(
                view.selected.id,
                Some(active.id),
                "background restart must not steal selection"
            );
            assert_eq!(
                view.sessions
                    .entries
                    .iter()
                    .find(|meta| meta.id == target.id)
                    .expect("target")
                    .status,
                SessionStatus::Idle
            );
        });
    })
    .expect("background restart receipt");
    assert!(
        fixture.commands.try_recv().is_err(),
        "no prompt or selection command"
    );
}

#[gpui_kit::test]
fn rows_context_pin_and_rename_target_without_selecting_on_open(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        for label in [
            crate::i18n::tr!("session-rename"),
            crate::i18n::tr!("session-bookmark"),
            crate::i18n::tr!("session-delete"),
        ] {
            assert!(window.within(label).find("shortcut").visible());
        }
        assert!(fixture.commands.try_recv().is_err());
        window.press("down", cx);
        window.press("enter", cx);
        assert!(fixture.view.read(cx).navigation.pinned.contains(&target.id));
        assert!(!fixture.view.read(cx).navigation.pinned.contains(&active.id));
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("pin correct row without RPC");
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(
            fixture
                .view
                .read(cx)
                .navigation
                .bookmarks
                .contains(&target.id)
        );
        assert!(
            !fixture
                .view
                .read(cx)
                .navigation
                .bookmarks
                .contains(&active.id)
        );
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("bookmark the context-menu target without selecting or sending RPCs");
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(fixture.view.read(cx).renaming_thread(target.id));
        while let Ok(command) = fixture.commands.try_recv() {
            assert!(!matches!(command, Command::Prompt { .. }));
        }
    })
    .expect("rename correct row");
}

#[gpui_kit::test]
fn rows_context_archive_targets_clicked_row(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        for _ in 0..4 {
            window.press("down", cx);
        }
        window.press("enter", cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Archive { session, archived: true }) if session == target.id
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("menu archives clicked row only");
}

#[gpui_kit::test]
fn rows_context_delete_opens_confirmation_without_rpc(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.click(crate::i18n::tr!("session-delete"), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        assert!(
            fixture.commands.try_recv().is_err(),
            "delete requires confirmation"
        );
        assert!(
            !fixture
                .view
                .read(cx)
                .sessions
                .pending
                .deleting
                .contains(&target.id)
        );
    })
    .expect("request confirmation only");
    cx.run_until_parked();
    let confirmation = cx
        .windows()
        .into_iter()
        .find(|window| window.window_id() != fixture.window.window_id())
        .expect("delete confirmation window");
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("confirmation-window").visible());
    })
    .expect("independent confirmation");
}
