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
    let generation = Arc::new(AtomicU64::new(7));
    let worker = std::thread::spawn(move || worker::run(input, events, generation));
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
            let current = player.generation.clone();
            player.reset(cx);
            assert!(
                current.load(Ordering::Acquire) > generation,
                "reset invalidates in-flight device initialization immediately"
            );
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

#[test]
fn completion_clears_the_current_item_and_drop_invalidates_pending_playback() {
    let mut player = Player::new();
    player.id = Some("finished".into());
    player.state = State::Playing;
    player.apply(Event {
        generation: 0,
        state: State::Idle,
    });
    assert!(player.id.is_none());
    assert_eq!(player.state, State::Idle);
    let generation = player.generation.clone();
    drop(player);
    assert_eq!(generation.load(Ordering::Acquire), 1);
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn icon_controls_share_playback_across_views_and_disable_stop_after_completion(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{
        AppContext, Context, Render, Subscription, Window, px, size, test::TestWindowExt,
    };
    struct Views {
        player: Entity<Player>,
        _changed: Subscription,
    }
    impl Render for Views {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            gds_uim::el! {
                #div { flex; flex_col; } [
                    $content("timeline", "same-message", &wav(), &self.player, cx),
                    $content("preview", "same-message", &wav(), &self.player, cx),
                ]
            }
        }
    }
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let (commands, received) = mpsc::channel();
    let player = cx.new(|_| {
        let mut player = Player::new();
        player.commands = Some(commands);
        player
    });
    let window = cx.open_window(size(px(560.), px(240.)), |window, cx| {
        let view = cx.new(|cx| Views {
            player: player.clone(),
            _changed: cx.observe(&player, |_, _, cx| cx.notify()),
        });
        Root::new(view, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "audio-toggle-timeline",
            "audio-stop-timeline",
            "audio-toggle-preview",
            "audio-stop-preview",
        ] {
            assert_eq!(
                window.find(id).bounds().size,
                size(
                    px(crate::ui::COMPACT_CONTROL),
                    px(crate::ui::COMPACT_CONTROL)
                )
            );
        }
        window.click("audio-toggle-preview", cx);
        let Command::Play { generation, .. } = received.try_recv().expect("play") else {
            panic!("play")
        };
        player.update(cx, |player, cx| {
            player.apply(Event {
                generation,
                state: State::Playing,
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("audio-toggle-timeline").label(), Some("暂停"));
        assert_eq!(window.find("audio-toggle-preview").label(), Some("暂停"));
        window.click("audio-toggle-timeline", cx);
        assert!(matches!(received.try_recv(), Ok(Command::Pause(id)) if id == generation));
        window.render_frame(cx);
        window.click("audio-toggle-preview", cx);
        assert!(matches!(received.try_recv(), Ok(Command::Resume(id)) if id == generation));
        player.update(cx, |player, cx| {
            player.apply(Event {
                generation,
                state: State::Idle,
            });
            cx.notify();
        });
        window.render_frame(cx);
        window.click("audio-stop-timeline", cx);
        window.click("audio-stop-preview", cx);
        assert!(
            received.try_recv().is_err(),
            "completed audio has no stop action"
        );
        assert_eq!(window.find("audio-toggle-timeline").label(), Some("播放"));
    })
    .expect("shared audio controls");
}
