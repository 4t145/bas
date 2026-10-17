use super::*;
use std::time::Duration;

const COMPLETION_INTERVAL: Duration = Duration::from_millis(100);

struct Playback {
    generation: u64,
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
    device_error: Arc<std::sync::Mutex<Option<String>>>,
}

/// # Errors
/// Returns audio decoding or output-device initialization failures.
fn play(generation: u64, audio: &acp::AudioContent) -> Result<Playback, Error> {
    let source = decode(audio)?;
    let device_error = Arc::new(std::sync::Mutex::new(None));
    let capture = Arc::clone(&device_error);
    let mut stream = rodio::OutputStreamBuilder::from_default_device()?
        .with_error_callback(move |error| {
            *capture.lock().expect("audio device error lock") = Some(error.to_string());
        })
        .open_stream_or_fallback()?;
    stream.log_on_drop(false);
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.append(source);
    Ok(Playback {
        generation,
        sink,
        _stream: stream,
        device_error,
    })
}

pub(super) fn run(
    commands: mpsc::Receiver<Command>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut playback: Option<Playback> = None;
    loop {
        let command = if playback.is_some() {
            match commands.recv_timeout(COMPLETION_INTERVAL) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        match command {
            Some(Command::Play { generation, audio }) => {
                playback.take();
                let state = match play(generation, &audio) {
                    Ok(active) => {
                        playback = Some(active);
                        State::Playing
                    }
                    Err(error) => State::Failed(error.to_string()),
                };
                if events.send(Event { generation, state }).is_err() {
                    break;
                }
            }
            Some(Command::Pause(generation)) => {
                if let Some(active) = &playback
                    && active.generation == generation
                {
                    active.sink.pause();
                }
            }
            Some(Command::Resume(generation)) => {
                if let Some(active) = &playback
                    && active.generation == generation
                {
                    active.sink.play();
                }
            }
            Some(Command::Stop) => {
                playback.take();
            }
            None => {}
        }
        let device_error = playback.as_ref().and_then(|active| {
            active
                .device_error
                .lock()
                .expect("audio device error lock")
                .take()
        });
        if let Some(error) = device_error {
            let failed = playback.take().expect("failed audio");
            if events
                .send(Event {
                    generation: failed.generation,
                    state: State::Failed(error),
                })
                .is_err()
            {
                break;
            }
        } else if playback.as_ref().is_some_and(|active| active.sink.empty()) {
            let finished = playback.take().expect("completed audio");
            if events
                .send(Event {
                    generation: finished.generation,
                    state: State::Idle,
                })
                .is_err()
            {
                break;
            }
        }
    }
    // Dropping the sink and stream stops playback, including when its window closes.
}
