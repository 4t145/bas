//! The agent backend linked into this daemon.
//!
//! BASR is meant to embed an agent implemented in this crate. That agent does
//! not exist yet, so [`Driver`] launches [`Echo`]: a deterministic placeholder
//! that streams the prompt back and commits it as a message. It keeps the
//! daemon, the CLI, and every client testable end to end before the real agent
//! lands, and it is the integration point to replace.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::model::{
    AgentId, ContentBlock, Delta, DeltaBlock, Message, MessageId, Role, SessionId, StopReason,
    TurnId,
};

use super::{Agent, Error, Kind, LaunchRequest, Update, driver::Driver as DriverTrait};

/// Delay between streamed chunks, so attached clients can observe streaming.
const CHUNK_DELAY: Duration = Duration::from_millis(10);
/// Characters per streamed chunk.
const CHUNK_SIZE: usize = 24;

/// Launches in-process agents.
#[derive(Default)]
pub struct Driver;

impl Driver {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DriverTrait for Driver {
    fn kind(&self) -> Kind {
        Kind::Embedded
    }

    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, Error> {
        let agent = Echo {
            agent: request.spec.id.clone(),
            session: request.session,
            cwd: request.cwd,
            title: Mutex::new(request.title),
            updates: request.updates,
            cancelled: Arc::new(AtomicBool::new(false)),
        };

        Ok(Box::new(agent))
    }
}

/// The placeholder in-process agent.
struct Echo {
    agent: AgentId,
    session: SessionId,
    cwd: Option<PathBuf>,
    title: Mutex<Option<String>>,
    updates: mpsc::UnboundedSender<Update>,
    cancelled: Arc<AtomicBool>,
}

impl Echo {
    fn describe(&self) -> String {
        let cwd = self
            .cwd
            .as_ref()
            .map_or_else(|| "<none>".to_owned(), |cwd| cwd.display().to_string());
        format!("echo agent: session {}, cwd {cwd}", self.session)
    }
}

#[async_trait]
impl Agent for Echo {
    async fn prompt(&self, content: Vec<ContentBlock>) -> Result<TurnId, Error> {
        let turn = TurnId::new();
        let message = MessageId::new();
        let reply = format!(
            "{}\n\n{}",
            self.describe(),
            content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        );

        let updates = self.updates.clone();
        let cancelled = Arc::clone(&self.cancelled);
        cancelled.store(false, Ordering::SeqCst);

        tokio::spawn(async move {
            if updates.send(Update::TurnStarted { turn }).is_err() {
                return;
            }

            for chunk in chunk_text(&reply) {
                if cancelled.load(Ordering::SeqCst) {
                    let _ = updates.send(Update::TurnFinished {
                        turn,
                        stop: StopReason::Cancelled,
                    });
                    return;
                }

                let delta = Update::Delta(Delta {
                    message,
                    role: Role::Agent,
                    block: DeltaBlock::Text { text: chunk },
                });
                if updates.send(delta).is_err() {
                    return;
                }
                tokio::time::sleep(CHUNK_DELAY).await;
            }

            let committed = Message::with_id(message, Role::Agent, vec![ContentBlock::text(reply)]);
            let _ = updates.send(Update::Message(committed));
            let _ = updates.send(Update::TurnFinished {
                turn,
                stop: StopReason::EndTurn,
            });
        });

        Ok(turn)
    }

    async fn cancel(&self) -> Result<(), Error> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn set_title(&self, title: &str) -> Result<(), Error> {
        *self
            .title
            .lock()
            .expect("the echo agent title mutex is never poisoned") = Some(title.to_owned());
        self.updates
            .send(Update::Title(title.to_owned()))
            .map_err(|_| Error::backend(self.agent.clone(), "the session is gone"))
    }

    async fn shutdown(&self) -> Result<(), Error> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for character in text.chars() {
        current.push(character);
        if current.chars().count() >= CHUNK_SIZE {
            chunks.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}
