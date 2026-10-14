use super::*;

fn catalog(name: &str) -> acp::AvailableCommandsUpdate {
    acp::AvailableCommandsUpdate::new(vec![acp::AvailableCommand::new(name, "description")])
}

#[test]
fn command_catalog_uses_revision_guarded_session_metadata() {
    let mut conversation = conversation();
    assert!(conversation.commands().is_empty());
    let mut snapshot = conversation.session.clone();
    snapshot.revision = Cursor::from_raw(10);
    snapshot.available_commands = Some(catalog("snapshot"));
    conversation.merge_meta(&snapshot);
    assert_eq!(conversation.commands()[0].name, "snapshot");
    conversation.apply(Event::new(
        snapshot.id,
        Cursor::from_raw(2),
        EventKind::Acp(acp::SessionUpdate::AvailableCommandsUpdate(catalog("old"))),
    ));
    assert_eq!(conversation.commands()[0].name, "snapshot");
    conversation.apply(Event::new(
        snapshot.id,
        Cursor::from_raw(11),
        EventKind::Acp(acp::SessionUpdate::AvailableCommandsUpdate(catalog("live"))),
    ));
    assert_eq!(conversation.commands()[0].name, "live");
    conversation.merge_meta(&snapshot);
    assert_eq!(conversation.commands()[0].name, "live");
    snapshot.revision = Cursor::from_raw(12);
    snapshot.available_commands = None;
    conversation.merge_meta(&snapshot);
    assert!(conversation.commands().is_empty());
}
