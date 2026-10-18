//! User-initiated playback of complete ACP audio blocks, never automatic replay.

use crate::ui::ContentRow;
use bas::acp;
use base64::Engine;
use gpui_kit::{
    AnyElement, App, Context, Entity, Task, TestSupportExt, assets::IconName, component::*, div,
    prelude::*,
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
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
    generation: Arc<AtomicU64>,
    scope: u64,
    state: State,
    commands: Option<mpsc::Sender<Command>>,
    _events: Option<Task<()>>,
}

impl Player {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            generation: Arc::new(AtomicU64::new(0)),
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
        let generation = Arc::clone(&self.generation);
        std::thread::Builder::new()
            .name("basg-audio".into())
            .spawn(move || worker::run(receiver, events, generation))?;
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
        if event.generation == self.generation.load(Ordering::Acquire) {
            if event.state == State::Idle {
                self.id = None;
            }
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
                Command::Pause(self.generation.load(Ordering::Acquire))
            }
            (State::Paused, true) => {
                self.state = State::Playing;
                Command::Resume(self.generation.load(Ordering::Acquire))
            }
            _ => {
                let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
                self.id = Some(id);
                self.state = State::Loading;
                Command::Play { generation, audio }
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
        self.generation.fetch_add(1, Ordering::AcqRel);
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

impl Drop for Player {
    fn drop(&mut self) {
        // Invalidate device initialization before the command sender is dropped.
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

pub(crate) fn content(
    id: &str,
    playback_id: &str,
    audio: &acp::AudioContent,
    player: &Entity<Player>,
    cx: &App,
) -> AnyElement {
    let state = player.read(cx);
    let scope = state.scope;
    let current = state.id.as_deref() == Some(playback_id);
    let playing = current && state.state == State::Playing;
    let loading = current && state.state == State::Loading;
    let can_stop =
        current && matches!(state.state, State::Loading | State::Playing | State::Paused);
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
    let toggle_id = playback_id.to_owned();
    let stop_id = playback_id.to_owned();
    let description = gds_uim::el! {
        #div { truncate; text_size: crate::ui::SMALL; } [
            $crate::i18n::tr!("resource-audio", type = audio.mime_type.clone()),
        ]
    };
    gds_uim::el! { #div {
        id: $format!("audio-{id}"); test_support; flex; flex_col; gap_2; p_3;
        border_1; border_color: $cx.theme().border; rounded_md;
    } [
        ContentRow($format!("audio-row-{id}"), $description) { icon: IconName::Volume2; } [
            #crate::ui::icon_action($format!("audio-toggle-{id}"),
                $(if playing { IconName::Pause } else { IconName::Play }),
                $(if playing { crate::i18n::tr!("audio-pause") } else { crate::i18n::tr!("audio-play") })) {
                loading: loading;
                on_click: $move |_, _, cx| toggle.update(cx, |player, cx| player.toggle(scope, toggle_id.clone(), Arc::clone(&audio), cx));
            },
            #crate::ui::icon_action($format!("audio-stop-{id}"), IconName::Square, $crate::i18n::tr!("audio-stop")) {
                disabled: $!can_stop;
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
