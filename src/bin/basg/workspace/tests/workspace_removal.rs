use super::archive::archived;
use super::*;

fn archive_requests(fixture: &mut Fixture) -> HashSet<SessionId> {
    std::iter::from_fn(|| fixture.commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Archive {
                session,
                archived: true,
            } => Some(session),
            Command::Foreground(_) | Command::Watch { .. } | Command::Detach => None,
            _ => panic!("removing a workspace must only archive and change navigation"),
        })
        .collect()
}

#[gpui_kit::test]
fn workspace_removal_archives_all_members_ignoring_search_and_preserves_data(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.cwd = Some("/one".into());
    first.status = SessionStatus::Working;
    let mut second = session();
    second.cwd = first.cwd.clone();
    let mut other = session();
    other.cwd = Some("/one/child".into());
    attach(&fixture, other.clone(), cx);
    attach(&fixture, second.clone(), cx);
    attach(&fixture, first.clone(), cx);
    let state = tempfile::tempdir().expect("window state");
    let state_path = state.path().join("window.json");
    let confirmation = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.attach_restore(Some(workspace_navigation::restore::Store::open(
                    state_path.clone(),
                )));
                view.navigation.restore.on_connect = false;
                view.navigation.search.update(cx, |input, cx| {
                    input.set_value("only one visible", window, cx)
                });
                view.composer
                    .input
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
                let before = cx.windows();
                view.request_remove_workspace(Some("/one".into()), cx);
                cx.windows()
                    .into_iter()
                    .find(|window| !before.contains(window))
                    .expect("confirmation")
            })
        })
        .expect("request workspace removal");
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm");
    cx.run_until_parked();
    assert_eq!(
        archive_requests(&mut fixture),
        HashSet::from([first.id, second.id])
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Archived {
                    session: first.id,
                    result: Ok(archived(first.clone(), true, 2).into()),
                },
                window,
                cx,
            );
            assert!(!view.navigation.hidden.contains(&Some("/one".into())));
            view.update(
                Update::Archived {
                    session: second.id,
                    result: Ok(archived(second.clone(), true, 2).into()),
                },
                window,
                cx,
            );
            assert!(view.navigation.removing.is_empty());
            assert!(view.navigation.hidden.contains(&Some("/one".into())));
            assert_eq!(view.selected.id, Some(other.id));
            assert_eq!(view.sessions.entries.len(), 3);
            assert_eq!(view.sessions.drafts[&first.id].text, "keep draft");
            assert!(
                view.sessions
                    .entries
                    .iter()
                    .find(|s| s.id == first.id)
                    .expect("archived first")
                    .archived
            );
            assert_eq!(
                view.sessions
                    .entries
                    .iter()
                    .find(|s| s.id == first.id)
                    .expect("running first")
                    .status,
                SessionStatus::Working
            );
            let visible = view.visible_sessions();
            assert!(
                view.grouped_threads(&visible, "")
                    .iter()
                    .all(|g| g.directory != Some("/one".into()))
            );
            view.navigation.view = workspace_navigation::View::Archived;
            let archived = view.visible_sessions();
            assert_eq!(
                view.grouped_threads(&archived, "")
                    .iter()
                    .find(|g| g.directory == Some("/one".into()))
                    .expect("archive group")
                    .threads
                    .len(),
                2
            );
        });
    })
    .expect("archive receipts");
    assert!(
        workspace_navigation::restore::Store::open(state_path)
            .hidden_workspaces()
            .contains(&Some("/one".into()))
    );
    assert!(archive_requests(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn partial_workspace_archive_keeps_the_group_and_retry_only_targets_unarchived_members(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.remove_workspace(first.cwd.clone(), window, cx);
            view.update(
                Update::Archived {
                    session: first.id,
                    result: Ok(archived(first.clone(), true, 2).into()),
                },
                window,
                cx,
            );
            view.update(
                Update::Archived {
                    session: second.id,
                    result: Err("could not archive".into()),
                },
                window,
                cx,
            );
            assert!(view.navigation.removing.is_empty());
            assert!(!view.navigation.hidden.contains(&first.cwd));
            assert_eq!(view.selected.id, Some(second.id));
        });
    })
    .expect("partial failure");
    assert_eq!(
        archive_requests(&mut fixture),
        HashSet::from([first.id, second.id])
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.remove_workspace(first.cwd.clone(), window, cx)
        });
    })
    .expect("retry");
    assert_eq!(archive_requests(&mut fixture), HashSet::from([second.id]));
}

#[gpui_kit::test]
fn empty_workspace_can_be_removed_and_explicitly_readded_without_losing_its_local_draft(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.new_thread(Some("/empty".into()), window, cx);
            view.backend.connected = true;
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("local draft", window, cx));
        });
        while fixture.commands.try_recv().is_ok() {}
        fixture.view.update(cx, |view, cx| {
            view.remove_workspace(Some("/empty".into()), window, cx);
            assert!(view.navigation.hidden.contains(&Some("/empty".into())));
            assert_eq!(
                view.navigation.drafts.contents[&Some("/empty".into())].text,
                "local draft"
            );
            view.backend.connected = false;
            view.new_thread(Some("/empty".into()), window, cx);
            assert!(!view.navigation.hidden.contains(&Some("/empty".into())));
            assert_eq!(view.composer.input.read(cx).value(), "local draft");
        });
    })
    .expect("remove and readd local workspace");
    assert!(archive_requests(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn closed_connection_clears_workspace_removal_without_hiding_unarchived_sessions(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let first = session();
    attach(&fixture, first.clone(), cx);
    fixture.commands.close();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.remove_workspace(first.cwd.clone(), window, cx);
            assert!(view.navigation.removing.is_empty());
            assert!(!view.navigation.hidden.contains(&first.cwd));
            assert!(!view.sessions.entries[0].archived);
        });
    })
    .expect("closed transport");
}

#[gpui_kit::test]
fn restoring_a_removed_launch_directory_does_not_create_a_replacement_session(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.selected.detach();
            view.selected.waiting_for_agent_selection = false;
            view.navigation
                .hidden
                .insert(view.navigation.selected_directory.clone());
            view.navigation.restore.on_connect = true;
            view.restore_thread(window, cx);
            view.prepare_session(cx);
            assert!(view.selected.waiting_for_agent_selection);
            assert!(view.selected.id.is_none());
            assert_eq!(view.navigation.selected_directory, None);
        });
    })
    .expect("restore removed launch directory");
    assert!(archive_requests(&mut fixture).is_empty());
}
