//! Deterministic echo backend used to exercise the daemon before a real embedded agent exists.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use basr::{
    acp,
    model::{AgentId, ContentBlock, SessionId, StopReason, TurnId},
};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    Agent, Error, Kind, LaunchRequest, TurnUpdate, Update,
    driver::{Driver as DriverTrait, spawn_worker},
};

const CHUNK_DELAY: Duration = Duration::from_millis(10);
const CHUNK_SIZE: usize = 24;

#[derive(Default)]
pub struct Driver;

impl Driver {
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
        Ok(Box::new(Echo {
            agent: request.spec.id.clone(),
            session: request.session,
            cwd: request.cwd,
            updates: request.updates,
            cancel: CancellationToken::new(),
            task: None,
        }))
    }

    // Echo retains no conversation history: restoring only reattaches the
    // daemon-owned identity and workspace, not prior-turn model context.
    async fn restore(
        &self,
        request: LaunchRequest,
        _acp_session_id: String,
    ) -> Result<Box<dyn Agent>, Error> {
        self.launch(request).await
    }
}

struct Echo {
    agent: AgentId,
    session: SessionId,
    cwd: PathBuf,
    updates: mpsc::Sender<Update>,
    cancel: CancellationToken,
    task: Option<AbortOnDropHandle<()>>,
}

#[async_trait]
impl Agent for Echo {
    fn prompt(&mut self, turn: TurnId, request: acp::PromptRequest) -> Result<(), Error> {
        let content = request.prompt;
        if content
            .iter()
            .any(|block| !matches!(block, ContentBlock::Text(_)))
        {
            return Err(Error::Unsupported {
                agent: self.agent.clone(),
                operation: "non-text prompts",
            });
        }
        let text = content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text),
                _ => None,
            })
            .collect::<String>();
        let reply = format!(
            "echo agent: session {}, cwd {}\n\n{text}",
            self.session,
            self.cwd.display()
        );
        let updates = self.updates.clone();
        self.cancel = CancellationToken::new();
        let cancel = self.cancel.clone();
        self.task = Some(spawn_worker(updates.clone(), async move {
            let mut stop = StopReason::EndTurn;
            for chunk in chunk_text(&reply) {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => { stop = StopReason::Cancelled; break; },
                    () = tokio::time::sleep(CHUNK_DELAY) => {},
                }
                let update =
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(chunk.into()));
                if updates
                    .send(Update::Turn {
                        turn,
                        update: TurnUpdate::Acp(update),
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            let _ = updates
                .send(Update::Turn {
                    turn,
                    update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(stop))),
                })
                .await;
            Ok(())
        }));
        Ok(())
    }

    fn cancel(&mut self, _request: acp::CancelNotification) -> Result<(), Error> {
        self.cancel.cancel();
        Ok(())
    }

    async fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

fn chunk_text(text: &str) -> Vec<String> {
    let mut chars = text.chars();
    let mut chunks = Vec::new();
    loop {
        let chunk: String = chars.by_ref().take(CHUNK_SIZE).collect();
        if chunk.is_empty() {
            return chunks;
        }
        chunks.push(chunk);
    }
}
