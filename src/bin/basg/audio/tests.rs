use super::*;
use rodio::Source;

fn wav() -> acp::AudioContent {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend(44u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes()); // PCM
    bytes.extend(1u16.to_le_bytes()); // mono
    bytes.extend(8_000u32.to_le_bytes());
    bytes.extend(16_000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(8u32.to_le_bytes());
    bytes.extend([0u8; 8]);
    acp::AudioContent::new(
        base64::engine::general_purpose::STANDARD.encode(bytes),
        "audio/wav",
    )
}

#[test]
fn decodes_native_audio_without_opening_a_device() {
    let decoded = decode(&wav()).expect("WAV");
    assert_eq!(decoded.sample_rate(), 8_000);
    assert_eq!(decoded.channels(), 1);
    assert_eq!(decoded.collect::<Vec<_>>(), vec![0.; 4]);
}

#[test]
fn malformed_data_and_unsupported_files_are_errors() {
    assert!(matches!(
        decode(&acp::AudioContent::new("!!!", "audio/wav")),
        Err(Error::Base64(_))
    ));
    assert!(matches!(
        decode(&acp::AudioContent::new(
            base64::engine::general_purpose::STANDARD.encode(b"not audio"),
            "audio/wav",
        )),
        Err(Error::Decoder(_))
    ));
}

#[test]
fn worker_reports_bad_audio_and_exits_when_its_owner_disconnects() {
    let (commands, input) = mpsc::channel();
    let (events, mut output) = tokio::sync::mpsc::unbounded_channel();
    let worker = std::thread::spawn(move || worker::run(input, events));
    commands
        .send(Command::Play {
            generation: 7,
            audio: Arc::new(acp::AudioContent::new("!!!", "audio/wav")),
        })
        .expect("play");
    let event = output.blocking_recv().expect("error");
    assert_eq!(event.generation, 7);
    assert!(matches!(event.state, State::Failed(_)));
    drop(commands);
    worker.join().expect("worker stops");
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn play_pause_stop_and_scope_changes_never_autoplay_or_accept_late_events(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::AppContext;
    let (commands, received) = mpsc::channel();
    let player = cx.new(|_| {
        let mut player = Player::new();
        player.commands = Some(commands);
        player
    });
    assert!(
        received.try_recv().is_err(),
        "constructing a player must not play"
    );
    cx.update(|cx| {
        player.update(cx, |player, cx| {
            let scope = player.scope;
            let audio = Arc::new(wav());
            player.toggle(scope, "first".into(), audio.clone(), cx);
            let Command::Play { generation, .. } = received.try_recv().expect("play") else {
                panic!("play")
            };
            player.apply(Event {
                generation,
                state: State::Playing,
            });
            player.toggle(scope, "first".into(), audio.clone(), cx);
            assert!(matches!(received.try_recv(), Ok(Command::Pause(id)) if id == generation));
            player.toggle(scope, "first".into(), audio.clone(), cx);
            assert!(matches!(received.try_recv(), Ok(Command::Resume(id)) if id == generation));
            player.reset(cx);
            assert!(matches!(received.try_recv(), Ok(Command::Stop)));
            player.apply(Event {
                generation,
                state: State::Playing,
            });
            player.toggle(scope, "stale card".into(), audio, cx);
            assert!(received.try_recv().is_err());
            assert_eq!(player.state, State::Idle);
            assert!(player.id.is_none());
        })
    });
}
