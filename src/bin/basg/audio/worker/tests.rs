use super::*;

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
