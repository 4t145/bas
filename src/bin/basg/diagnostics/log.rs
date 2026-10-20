//! Bounded, process-local diagnostics. No subscriber installation or disk I/O.

use std::{
    collections::VecDeque,
    fmt::{self, Write as _},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::watch;
use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context};

const MAX_ENTRIES: usize = 2_000;
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_TARGET_BYTES: usize = 1024;
const TRUNCATED: &str = " … [truncated]";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

impl From<&tracing::Level> for Level {
    fn from(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub id: u64,
    pub time: SystemTime,
    pub level: Level,
    pub target: String,
    pub message: String,
}

impl Entry {
    /// Copyable plain text; timestamps are seconds since the Unix epoch.
    pub(crate) fn text(&self) -> String {
        let (sign, elapsed) = match self.time.duration_since(UNIX_EPOCH) {
            Ok(elapsed) => ("", elapsed),
            Err(error) => ("-", error.duration()),
        };
        plain_text(&format!(
            "{sign}{}.{:03} {} {}: {}",
            elapsed.as_secs(),
            elapsed.subsec_millis(),
            self.level.as_str(),
            self.target,
            self.message
        ))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub entries: Vec<Entry>,
    /// Entries evicted by retention since the last clear.
    pub evicted: u64,
}

#[derive(Default)]
struct State {
    revision: u64,
    next_id: u64,
    entries: VecDeque<Entry>,
    message_bytes: usize,
    evicted: u64,
}

struct Shared {
    state: Mutex<State>,
    updates: watch::Sender<u64>,
}

#[derive(Clone)]
pub(crate) struct Store {
    shared: Arc<Shared>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                updates: watch::channel(0).0,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // Diagnostics remain available even after a producer panics.
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Freeform messages must already be safe to log; only structured tracing
    /// fields receive name-based redaction.
    pub(crate) fn record(
        &self,
        level: Level,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        let target = plain_text_bounded(&target.into(), MAX_TARGET_BYTES);
        let message = plain_text_bounded(&message.into(), MAX_RECORD_BYTES);
        let mut state = self.state();
        state.next_id += 1;
        let id = state.next_id;
        state.message_bytes += message.len();
        state.entries.push_back(Entry {
            id,
            time: SystemTime::now(),
            level,
            target,
            message,
        });
        while state.entries.len() > MAX_ENTRIES || state.message_bytes > MAX_MESSAGE_BYTES {
            if let Some(entry) = state.entries.pop_front() {
                state.message_bytes -= entry.message.len();
                state.evicted += 1;
            }
        }
        self.publish(&mut state);
        id
    }

    fn publish(&self, state: &mut State) {
        state.revision += 1;
        // Publish inside the state lock, including when no receiver exists yet.
        self.shared.updates.send_replace(state.revision);
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.state();
        Snapshot {
            revision: state.revision,
            entries: state.entries.iter().cloned().collect(),
            evicted: state.evicted,
        }
    }

    /// Clears retained entries and eviction count, but never reuses entry IDs.
    pub(crate) fn clear(&self) {
        let mut state = self.state();
        state.entries.clear();
        state.message_bytes = 0;
        state.evicted = 0;
        self.publish(&mut state);
    }

    /// Subscribe before taking an initial snapshot to avoid missing an update.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.shared.updates.subscribe()
    }

    pub(crate) fn layer<S: Subscriber>(&self) -> impl Layer<S> + use<S> {
        StoreLayer(self.clone())
    }
}

struct StoreLayer(Store);

impl<S: Subscriber> Layer<S> for StoreLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let metadata = event.metadata();
        let message = if fields.message.is_empty() {
            fields.other
        } else if fields.other.is_empty() {
            fields.message
        } else {
            format!("{} {}", fields.message, fields.other)
        };
        self.0
            .record(metadata.level().into(), metadata.target(), message);
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    other: String,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.record_value(field.name(), format_args!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field.name(), format_args!("{value}"));
    }
}

impl Fields {
    fn record_value(&mut self, name: &str, value: fmt::Arguments<'_>) {
        if sensitive_field(name) {
            self.append_field(name, "[redacted]");
        } else {
            // Formatting happens outside the store lock, including user Debug impls.
            let mut output = LimitedText::new(MAX_RECORD_BYTES);
            // A full collector returns fmt::Error to stop cooperative formatters.
            // Do not unwrap: user formatters can also return errors themselves.
            let _ = output.write_fmt(value);
            let value = output.text;
            if name == "message" {
                self.message = value;
            } else {
                self.append_field(name, &value);
            }
        }
    }

    fn append_field(&mut self, name: &str, value: &str) {
        if self.other.len() >= MAX_RECORD_BYTES {
            return;
        }
        if !self.other.is_empty() {
            self.other.push(' ');
        }
        self.other.push_str(name);
        self.other.push('=');
        self.other.push_str(value);
        self.other = bounded(std::mem::take(&mut self.other), MAX_RECORD_BYTES);
    }
}

fn sensitive_field(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "authorization",
        "token",
        "apikey",
        "password",
        "passwd",
        "privatekey",
        "secret",
        "certificate",
        "cert",
        "pem",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

fn bounded(mut text: String, max_bytes: usize) -> String {
    if text.len() > max_bytes {
        let mut end = max_bytes - TRUNCATED.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(TRUNCATED);
    }
    text
}

struct LimitedText {
    text: String,
    max_bytes: usize,
    truncated: bool,
}

impl LimitedText {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::new(),
            max_bytes,
            truncated: false,
        }
    }
}

impl fmt::Write for LimitedText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Err(fmt::Error);
        }
        let remaining = self.max_bytes - self.text.len();
        if text.len() <= remaining {
            self.text.push_str(text);
            return Ok(());
        }
        self.text
            .push_str(&text[..text.floor_char_boundary(remaining)]);
        self.text.truncate(
            self.text
                .floor_char_boundary(self.max_bytes - TRUNCATED.len()),
        );
        self.text.push_str(TRUNCATED);
        self.truncated = true;
        Err(fmt::Error)
    }
}

/// Strip terminal escape sequences (CSI, OSC and short ESC forms) and controls,
/// preserving newlines and tabs in diagnostics.
fn plain_text(text: &str) -> String {
    plain_text_bounded(text, usize::MAX)
}

fn plain_text_bounded(text: &str, max_bytes: usize) -> String {
    let mut output = LimitedText::new(max_bytes);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if (!ch.is_control() || ch == '\n' || ch == '\t') && output.write_char(ch).is_err() {
            break;
        }
    }
    output.text
}

#[cfg(test)]
mod tests;
