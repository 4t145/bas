use super::*;

fn open_workspace_choices(fixture: &Fixture, cx: &mut TestAppContext) -> gpui_kit::AnyWindowHandle {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("add-workspace-button", cx);
    })
    .expect("open workspace choices");
    cx.executor().run_until_parked();
    cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .menu
            .as_ref()
            .expect("workspace choices menu")
            .window
            .expect("native workspace choices surface")
    })
}

fn choose_local_workspace(fixture: &Fixture, cx: &mut TestAppContext) {
    let menu = open_workspace_choices(fixture, cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .expect("choose local workspace");
    cx.executor().run_until_parked();
    assert!(
        !cx.did_prompt_for_paths(),
        "wait for the native activation frame"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .expect("deliver the main window activation frame");
    cx.executor().run_until_parked();
    assert!(cx.did_prompt_for_paths());
}

fn assert_workspace_commands(
    commands: &mut mpsc::UnboundedReceiver<Command>,
    expected_detaches: usize,
) {
    let mut detaches = 0;
    while let Ok(command) = commands.try_recv() {
        match command {
            Command::Foreground(_) => {}
            Command::Detach => detaches += 1,
            _ => panic!(
                "choosing a workspace must not create, send, archive, delete or stop a thread"
            ),
        }
    }
    assert_eq!(detaches, expected_detaches);
}

#[gpui_kit::test]
fn local_folder_selection_preserves_threads_and_restores_workspace_drafts(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut existing = session();
    existing.cwd = Some("/workspace/source".into());
    existing.title = Some("existing thread".into());
    existing.status = SessionStatus::Working;
    attach(&fixture, existing.clone(), cx);
    let folder = tempfile::tempdir().expect("local workspace folder");
    let target = groups::normalize(folder.path());
    let file = tempfile::NamedTempFile::new().expect("draft attachment");
    std::fs::write(file.path(), "keep attachment").expect("attachment contents");
    let attachment = Attachment::load(file.path()).expect("load attachment");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("source draft", window, cx);
            });
            view.attachments = vec![attachment.clone()];
            view.outboxes
                .entry(existing.id)
                .or_default()
                .enqueue(Draft {
                    text: "source queued prompt".into(),
                    attachments: Vec::new(),
                });
            view.new_drafts.insert(
                Some(target.clone()),
                Draft {
                    text: "target local draft".into(),
                    attachments: Vec::new(),
                },
            );
            view.new_draft_metadata
                .entry(Some(target.clone()))
                .or_default()
                .title = Some("target draft title".into());
            cx.notify();
        });
    })
    .expect("prepare thread and local draft");
    while fixture.commands.try_recv().is_ok() {}
    choose_local_workspace(&fixture, cx);
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files);
        assert!(options.directories);
        assert!(!options.multiple);
        assert_eq!(options.prompt.as_deref(), Some("选择工作区文件夹"));
        Some(vec![target.clone()])
    });
    cx.executor().run_until_parked();
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.active, None);
        assert_eq!(view.selected_directory.as_ref(), Some(&target));
        assert_eq!(view.sessions.len(), 1);
        assert_eq!(view.sessions[0].id, existing.id);
        assert_eq!(view.sessions[0].title, existing.title);
        assert_eq!(view.sessions[0].status, SessionStatus::Working);
        let source = view.drafts.get(&existing.id).expect("saved thread draft");
        assert_eq!(source.text, "source draft");
        assert_eq!(source.attachments.len(), 1);
        assert_eq!(source.attachments[0].id, attachment.id);
        let queue = view.outboxes.get(&existing.id).expect("retained queue");
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.front().expect("retained queued prompt").text,
            "source queued prompt"
        );
        assert_eq!(view.composer.read(cx).value(), "target local draft");
        assert_eq!(view.draft_title(), "target draft title");
        assert_eq!(
            view.directories
                .iter()
                .filter(|path| **path == target)
                .count(),
            1
        );
    });
    assert_workspace_commands(&mut fixture.commands, 1);
}

#[gpui_kit::test]
fn cancelling_folder_selection_keeps_the_current_thread_and_composition(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let existing = session();
    attach(&fixture, existing.clone(), cx);
    let (directories, local_drafts) = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("keep current composition", window, cx);
                });
                let local_drafts: HashMap<_, _> = view
                    .new_drafts
                    .iter()
                    .map(|(directory, draft)| (directory.clone(), draft.text.clone()))
                    .collect();
                (view.directories.clone(), local_drafts)
            })
        })
        .expect("prepare composition before opening folder chooser");
    while fixture.commands.try_recv().is_ok() {}
    choose_local_workspace(&fixture, cx);
    cx.simulate_path_prompt_response(|_| None);
    cx.executor().run_until_parked();
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.active, Some(existing.id));
        assert_eq!(view.directories, directories);
        assert_eq!(view.composer.read(cx).value(), "keep current composition");
        assert_eq!(
            view.new_drafts
                .iter()
                .map(|(directory, draft)| (directory.clone(), draft.text.clone()))
                .collect::<HashMap<_, _>>(),
            local_drafts
        );
        assert!(view.error.is_none());
    });
    assert_workspace_commands(&mut fixture.commands, 0);
}

#[gpui_kit::test]
fn selecting_an_existing_normalized_folder_keeps_one_workspace_and_both_drafts(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let folder = tempfile::tempdir().expect("existing workspace folder");
    let target = groups::normalize(folder.path());
    let mut existing = session();
    existing.cwd = Some(target.clone());
    attach(&fixture, existing.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.directories.push(target.clone());
            view.composer.update(cx, |input, cx| {
                input.set_value("existing thread draft", window, cx);
            });
            view.new_drafts.insert(
                Some(target.clone()),
                Draft {
                    text: "existing local draft".into(),
                    attachments: Vec::new(),
                },
            );
            cx.notify();
        });
    })
    .expect("prepare existing workspace with thread and local drafts");
    while fixture.commands.try_recv().is_ok() {}
    for selected in [target.join("."), target.clone()] {
        choose_local_workspace(&fixture, cx);
        cx.simulate_path_prompt_response(|_| Some(vec![selected]));
        cx.executor().run_until_parked();
        cx.update(|cx| {
            let view = fixture.view.read(cx);
            assert_eq!(view.selected_directory.as_ref(), Some(&target));
            assert_eq!(
                view.selected_directory
                    .as_ref()
                    .expect("selected local folder")
                    .as_os_str(),
                target.as_os_str(),
                "the chosen path is normalized before it is stored"
            );
            assert_eq!(view.sessions.len(), 1);
            assert_eq!(view.sessions[0].id, existing.id);
            assert_eq!(
                view.drafts
                    .get(&existing.id)
                    .expect("retained existing thread draft")
                    .text,
                "existing thread draft"
            );
            assert_eq!(view.composer.read(cx).value(), "existing local draft");
            assert_eq!(
                view.directories
                    .iter()
                    .filter(|path| **path == target)
                    .count(),
                1
            );
            let sessions = view.visible_sessions();
            assert_eq!(
                view.grouped_threads(&sessions, "")
                    .iter()
                    .filter(|group| group.directory.as_ref() == Some(&target))
                    .count(),
                1
            );
        });
    }
    assert_workspace_commands(&mut fixture.commands, 2);
}

#[gpui_kit::test]
fn remote_workspace_choice_is_visible_and_disabled(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let existing = session();
    attach(&fixture, existing.clone(), cx);
    let (directories, selected_directory) = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("keep this remote choice draft", window, cx);
                });
                (view.directories.clone(), view.selected_directory.clone())
            })
        })
        .expect("prepare composition before remote choice");
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_workspace_choices(&fixture, cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        let local = window.within("popup-menu").find(0usize);
        let remote = window.within("popup-menu").find(1usize);
        assert!(local.visible());
        assert_eq!(local.label(), Some("本地文件夹…"));
        assert!(remote.visible());
        assert_eq!(remote.label(), Some("远程工作区（稍后支持）"));
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("remote workspace choice cannot be activated");
    cx.executor().run_until_parked();
    assert!(!cx.did_prompt_for_paths());
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.active, Some(existing.id));
        assert_eq!(view.sessions.len(), 1);
        assert_eq!(view.directories, directories);
        assert_eq!(view.selected_directory, selected_directory);
        assert_eq!(
            view.composer.read(cx).value(),
            "keep this remote choice draft"
        );
    });
    assert_workspace_commands(&mut fixture.commands, 0);
}
