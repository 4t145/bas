use super::*;
use bas::acp;

#[gpui_kit::test]
fn audio_attachment_requires_audio_not_embedded_context_and_preserves_the_draft(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("voice.wav");
    std::fs::write(&path, b"RIFF-fixture").expect("audio file");
    let audio = Attachment::load(&path).expect("attachment");
    let unsupported = session();
    attach(&fixture, unsupported.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            view.add_attachments(unsupported.id, vec![audio.clone()], window, cx);
            assert!(view.composer.attachments.is_empty());
            assert_eq!(view.composer.input.read(cx).value(), "keep draft");
            assert!(view.selected.error.is_some());
        });
    })
    .expect("unsupported attachment");
    let mut supported = session();
    supported.initialization = Some(
        acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new()
                    .prompt_capabilities(acp::PromptCapabilities::new().audio(true)),
            ),
    );
    attach(&fixture, supported.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.add_attachments(supported.id, vec![audio], window, cx);
            assert_eq!(view.composer.attachments.len(), 1);
            assert!(matches!(
                view.composer.attachments[0].content.as_ref(),
                ContentBlock::Audio(_)
            ));
            assert!(view.attachment_error(cx).is_none());
        });
    })
    .expect("native audio attachment");
}

#[gpui_kit::test]
fn received_audio_has_playback_controls_even_when_the_agent_does_not_accept_audio_input(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let id = fixture.view.update(cx, |view, cx| {
            assert!(!view.prompt_capabilities().audio);
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                            acp::ContentChunk::new(ContentBlock::Audio(acp::AudioContent::new(
                                "!!!",
                                "audio/wav",
                            ))),
                        )),
                    ),
                },
                window,
                cx,
            );
            view.selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation")
                .items[0]
                .id()
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(format!("audio-toggle-{id}-0")).label(),
            Some("播放")
        );
        assert_eq!(
            window.find(format!("audio-stop-{id}-0")).label(),
            Some("停止播放")
        );
        // Malformed data is not decoded until explicitly played; replay itself
        // must neither open a sound device nor fail the conversation.
    })
    .expect("audio controls");
}
