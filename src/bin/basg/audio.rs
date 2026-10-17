//! User-initiated playback of complete ACP audio blocks, never automatic replay.

use crate::ui::Button;
use bas::acp;
use base64::Engine;
use gpui_kit::{
    AnyElement, App, Context, Entity, Task, TestSupportExt,
    assets::IconName,
    component::{button::*, *},
    div,
    prelude::*,
};
use std::{
    io::Cursor,
    sync::{Arc, mpsc},
};

#[cfg(test)]
mod tests;
mod worker;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Invalid base64 audio: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Unsupported or invalid audio: {0}")]
    Decoder(#[from] rodio::decoder::DecoderError),
    #[error("Audio output is unavailable: {0}")]
    Output(#[from] rodio::StreamError),
}

/// # Errors
/// Rejects malformed base64 and audio formats the decoder cannot read.
fn decode(audio: &acp::AudioContent) -> Result<rodio::Decoder<Cursor<Vec<u8>>>, Error> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(&audio.data)?;
    Ok(rodio::Decoder::try_from(Cursor::new(bytes))?)
}

enum Command {
    Play {
        generation: u64,
        audio: Arc<acp::AudioContent>,
    },
    Pause(u64),
    Resume(u64),
    Stop,
}

struct Event {
    generation: u64,
    state: State,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Idle,
    Loading,
    Playing,
    Paused,
    Failed(String),
}

pub(crate) struct Player {
    id: Option<String>,
    generation: u64,
    scope: u64,
    state: State,
    commands: Option<mpsc::Sender<Command>>,
    _events: Option<Task<()>>,
}

impl Player {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            generation: 0,
            scope: 0,
            state: State::Idle,
            commands: None,
            _events: None,
        }
    }

    /// # Errors
    /// Returns the operating system error if the playback thread cannot start.
    fn connect(&mut self, cx: &mut Context<Self>) -> Result<(), std::io::Error> {
        if self.commands.is_some() {
            return Ok(());
        }
        let (commands, receiver) = mpsc::channel();
        let (events, mut updates) = tokio::sync::mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("basg-audio".into())
            .spawn(move || worker::run(receiver, events))?;
        self.commands = Some(commands);
        self._events = Some(cx.spawn(async move |player, cx| {
            while let Some(event) = updates.recv().await {
                if player
                    .update(cx, |player, cx| {
                        player.apply(event);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        Ok(())
    }

    fn apply(&mut self, event: Event) {
        if event.generation == self.generation {
            self.state = event.state;
        }
    }

    fn toggle(
        &mut self,
        scope: u64,
        id: String,
        audio: Arc<acp::AudioContent>,
        cx: &mut Context<Self>,
    ) {
        if scope != self.scope {
            return;
        }
        if let Err(error) = self.connect(cx) {
            self.id = Some(id);
            self.state = State::Failed(error.to_string());
            cx.notify();
            return;
        }
        let command = match (&self.state, self.id.as_deref() == Some(&id)) {
            (State::Playing, true) => {
                self.state = State::Paused;
                Command::Pause(self.generation)
            }
            (State::Paused, true) => {
                self.state = State::Playing;
                Command::Resume(self.generation)
            }
            _ => {
                self.generation += 1;
                self.id = Some(id);
                self.state = State::Loading;
                Command::Play {
                    generation: self.generation,
                    audio,
                }
            }
        };
        if self
            .commands
            .as_ref()
            .expect("audio worker connected")
            .send(command)
            .is_err()
        {
            self.commands = None;
            self.state = State::Failed(crate::i18n::tr!("audio-worker-ended").into());
        }
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Stop);
        }
        self.id = None;
        self.state = State::Idle;
        cx.notify();
    }

    pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
        self.scope += 1;
        self.stop(cx);
    }
}

pub(crate) fn content(
    id: &str,
    audio: &acp::AudioContent,
    player: &Entity<Player>,
    cx: &App,
) -> AnyElement {
    let state = player.read(cx);
    let scope = state.scope;
    let current = state.id.as_deref() == Some(id);
    let playing = current && state.state == State::Playing;
    let loading = current && state.state == State::Loading;
    let error = if current {
        match &state.state {
            State::Failed(error) => Some(error.clone()),
            _ => None,
        }
    } else {
        None
    };
    let audio = Arc::new(audio.clone());
    let toggle = player.clone();
    let stop = player.clone();
    let toggle_id = id.to_owned();
    let stop_id = id.to_owned();
    gds_uim::el! { #div {
        id: $format!("audio-{id}"); test_support; flex; flex_col; gap_2; p_3;
        border_1; border_color: $cx.theme().border; rounded_md;
    } [
        #div { flex; items_center; gap_2; } [
            Icon(IconName::Volume2) { small; },
            #div { flex_1; min_w_0; truncate; text_size: $crate::ui::SMALL; } [
                $crate::i18n::tr!("resource-audio", type = audio.mime_type.clone()),
            ],
            Button($format!("audio-toggle-{id}")) {
                ghost; small; icon: $(if playing { IconName::Pause } else { IconName::Play });
                label: $(if playing { crate::i18n::tr!("audio-pause") } else { crate::i18n::tr!("audio-play") });
                loading: loading;
                on_click: $move |_, _, cx| toggle.update(cx, |player, cx| player.toggle(scope, toggle_id.clone(), Arc::clone(&audio), cx));
            },
            Button($format!("audio-stop-{id}")) {
                ghost; small; icon: IconName::Square; label: $crate::i18n::tr!("audio-stop");
                disabled: $!current;
                on_click: $move |_, _, cx| stop.update(cx, |player, cx| {
                    if player.scope == scope && player.id.as_deref() == Some(&stop_id) { player.stop(cx); }
                });
            },
        ],
        ..$error.map(|error| gds_uim::el! {
            #div { text_size: $crate::ui::SMALL; text_color: $cx.theme().danger; } [
                $crate::i18n::tr!("audio-error", error = error),
            ]
        }),
    ] }.into_any_element()
}
