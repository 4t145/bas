use super::*;

#[test]
fn a_stop_or_new_play_during_preparation_keeps_the_old_sink_paused() {
    let (mixer, _output) = rodio::mixer::mixer(1, 8_000);
    let sink = rodio::Sink::connect_new(&mixer);
    sink.pause();
    sink.append(rodio::buffer::SamplesBuffer::new(1, 8_000, vec![0.25; 8]));
    let current = AtomicU64::new(1);
    current.fetch_add(1, Ordering::AcqRel);
    assert!(!start_if_current(1, &sink, &current));
    assert!(
        sink.is_paused(),
        "a cancelled initialization never starts output"
    );
    assert!(start_if_current(2, &sink, &current));
    assert!(!sink.is_paused());
}

#[test]
fn superseded_queued_playback_is_skipped_before_decoding() {
    let (commands, input) = mpsc::channel();
    let (events, mut output) = tokio::sync::mpsc::unbounded_channel();
    let current = Arc::new(AtomicU64::new(2));
    let audio = Arc::new(acp::AudioContent::new("!!!", "audio/wav"));
    commands
        .send(Command::Play {
            generation: 1,
            audio: audio.clone(),
        })
        .expect("stale play");
    commands
        .send(Command::Play {
            generation: 2,
            audio,
        })
        .expect("current play");
    drop(commands);
    run(input, events, current);
    let event = output.blocking_recv().expect("current decoding failure");
    assert_eq!(event.generation, 2);
    assert!(matches!(event.state, State::Failed(_)));
    assert!(
        output.blocking_recv().is_none(),
        "stale play never reached decoding or output"
    );
}
