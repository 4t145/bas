use super::*;
use bas::model::{RuntimeFailure, SessionSnapshot};

fn failed(mut snapshot: SessionSnapshot) -> SessionSnapshot {
    snapshot.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "disk full".into(),
    });
    snapshot
}

#[gpui_kit::test]
fn persistence_retry_waits_for_catalog_and_never_resubmits(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let snapshot = failed(SessionSnapshot::from(meta.clone()));
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.backend.agents.clone(), sessions: vec![snapshot],
            })), window, cx);
            view.retry_history(cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == meta.id));
            assert!(view.history_retry_disabled());
            view.retry_history(cx);
            assert!(fixture.commands.try_recv().is_err(), "one recovery at a time");

            view.update(Update::RestartedSession {
                session: meta.id, result: Err(Failure::Timeout),
            }, window, cx);
            assert!(!view.sessions.healthy(meta.id));
            assert!(!view.history_retry_disabled(), "failed recovery can be retried");
            view.retry_history(cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == meta.id));

            let mut recovered_meta = meta.clone();
            recovered_meta.revision = meta.revision.next();
            let recovered = SessionSnapshot::from(recovered_meta);
            view.update(Update::RestartedSession {
                session: meta.id, result: Ok(recovered.clone()),
            }, window, cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::Refresh)));
            assert!(!view.sessions.healthy(meta.id), "RPC receipt cannot supersede newer catalog state");
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.backend.agents.clone(), sessions: vec![recovered],
            })), window, cx);
            assert!(view.sessions.healthy(meta.id));
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == meta.id));
            assert!(fixture.commands.try_recv().is_err(), "recovery never sends a prompt");
        });
    }).expect("recovery UI");
}

#[gpui_kit::test]
fn runtime_failure_gates_current_and_background_without_rewriting_committed_state(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let mut current = session();
    current.status = SessionStatus::Working;
    current.revision = Cursor::from_raw(1);
    let background = session();
    attach(&fixture, current.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let healthy = SessionSnapshot::from(current.clone());
            let background = SessionSnapshot::from(background.clone());
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![healthy.clone(), background.clone()],
                })),
                window,
                cx,
            );
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("draft stays", window, cx));
            let partial = Event::new(
                current.id,
                Cursor::from_raw(2),
                EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                    bas::acp::ContentChunk::new("partial reply".into()),
                )),
            );
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: partial.into(),
                },
                window,
                cx,
            );
            for id in [current.id, background.meta.id] {
                view.sessions
                    .outboxes
                    .entry(id)
                    .or_default()
                    .enqueue(Draft {
                        text: "queued".into(),
                        attachments: Vec::new(),
                    });
            }
            let failed_current = failed(healthy.clone());
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![failed_current.clone(), failed(background.clone())],
                })),
                window,
                cx,
            );
            assert!(!view.can_send(cx));
            assert!(!view.working());
            assert!(
                view.composer_error(cx)
                    .expect("failure")
                    .contains("disk full")
            );
            assert_eq!(
                view.current().expect("committed").status,
                SessionStatus::Working
            );
            assert_eq!(view.current().expect("committed").revision.raw(), 2);
            let item_count = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation")
                .items
                .len();
            for id in [current.id, background.meta.id] {
                assert!(view.sessions.outboxes[&id].paused());
                assert_eq!(
                    view.sessions.outboxes[&id].front().expect("draft").text,
                    "queued"
                );
            }
            view.send_prompt(window, cx);
            view.flush_queue(cx);
            assert!(
                fixture.commands.try_recv().is_err(),
                "neither queue nor composer sends"
            );
            view.update(
                history_window(view.selected.epoch, healthy.clone(), Vec::new()),
                window,
                cx,
            );
            assert!(
                !view.sessions.healthy(current.id),
                "old healthy snapshot cannot clear fault"
            );
            assert_eq!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("partial retained")
                    .items
                    .len(),
                item_count
            );
            assert_eq!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("partial retained")
                    .cursor,
                2,
                "committed history cannot rewind the partial reply cursor"
            );
            assert_eq!(view.composer.input.read(cx).value().as_ref(), "draft stays");
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        current.id,
                        Cursor::from_raw(3),
                        EventKind::TitleChanged {
                            title: "replayed".into(),
                        },
                    )
                    .into(),
                },
                window,
                cx,
            );
            assert!(
                !view.sessions.healthy(current.id),
                "event replay cannot clear fault"
            );
            let mut restarted_meta = current.clone();
            restarted_meta.status = SessionStatus::Idle;
            restarted_meta.revision = Cursor::from_raw(4);
            let restarted = SessionSnapshot::from(restarted_meta);
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![restarted, SessionSnapshot::from(background.meta)],
                })),
                window,
                cx,
            );
            assert!(view.sessions.healthy(current.id));
            assert!(
                matches!(fixture.commands.try_recv(),
                Ok(Command::Watch { session, .. }) if session == current.id),
                "a recovered runtime reloads committed history without resending a prompt"
            );
            view.update(
                history_window(view.selected.epoch, failed_current, Vec::new()),
                window,
                cx,
            );
            assert!(
                view.sessions.healthy(current.id),
                "late previous runtime cannot replace new catalog runtime"
            );
            assert!(
                view.sessions.outboxes[&current.id].paused(),
                "restart never resumes paused queue"
            );
            assert!(fixture.commands.try_recv().is_err());
        });
    })
    .expect("runtime updates");
}
