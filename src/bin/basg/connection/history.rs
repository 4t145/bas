use super::*;
use std::collections::{HashMap, HashSet};

const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const BATCH_SIZE: usize = 64;
const WRITE_CAPACITY: usize = 256;

pub(super) struct Knowledge {
    index: bas::model::history::DependencyIndex,
    partial: bool,
    required_frontier: u64,
}

impl Default for Knowledge {
    fn default() -> Self {
        Self {
            index: Default::default(),
            partial: true,
            required_frontier: 0,
        }
    }
}

impl Knowledge {
    pub(super) fn require(&mut self, frontier: u64) {
        self.required_frontier = frontier;
    }

    pub(super) fn validate(&self, frontier: u64) -> Result<(), Failure> {
        if frontier < self.required_frontier {
            return Err(Failure::InvalidPayload(
                "refreshed history does not include the live dependency".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn reset(&mut self, window: &HistoryWindow) {
        self.index = Default::default();
        self.partial = window.before.is_some();
        for event in window.context.iter().chain(&window.events) {
            self.index.observe(event);
        }
    }

    pub(super) fn needs_reload(&self, event: &Event) -> bool {
        self.partial && self.index.may_reference_unloaded(event)
    }

    pub(super) fn observe(&mut self, event: &Event) {
        self.index.observe(event);
    }
}

#[derive(Clone)]
pub(super) struct Store {
    cache: Option<cache::Cache>,
    writes: mpsc::Sender<Write>,
}

enum Write {
    Window(Box<HistoryWindow>),
    Event(Box<Event>),
    Forget(SessionId),
    Flush,
}

impl Store {
    pub(super) fn new(cache: Option<cache::Cache>) -> (Self, JoinHandle<()>) {
        let (writes, receiver) = mpsc::channel(WRITE_CAPACITY);
        let task = tokio::spawn(write_loop(cache.clone(), receiver));
        (Self { cache, writes }, task)
    }

    pub(super) async fn window(&self, window: &HistoryWindow) {
        let _ = self
            .writes
            .send(Write::Window(Box::new(window.clone())))
            .await;
    }

    pub(super) async fn event(&self, event: &Event) {
        if !event.is_transient() {
            let _ = self
                .writes
                .send(Write::Event(Box::new(event.clone())))
                .await;
        }
    }

    pub(super) async fn flush(&self) {
        let _ = self.writes.send(Write::Flush).await;
    }

    pub(super) async fn forget(&self, session: SessionId) {
        let _ = self.writes.send(Write::Forget(session)).await;
    }

    pub(super) async fn latest(&self, session: SessionId) -> Option<HistoryWindow> {
        match self.cache.as_ref()?.load_latest(session).await {
            Ok(window) => window,
            Err(error) => {
                tracing::warn!(%session, %error, "Cannot read cached history");
                None
            }
        }
    }

    pub(super) async fn older(
        &self,
        mut client: Client,
        session: SessionId,
        before: u64,
    ) -> Result<HistoryWindow, Failure> {
        if let Some(cache) = &self.cache {
            match cache.load_before(session, before).await {
                Ok(Some(window)) => return Ok(window),
                Ok(None) => {}
                Err(error) => tracing::warn!(%session, %error, "Cannot read cached older history"),
            }
        }
        let window = client
            .history_window(session, Some(before), HISTORY_PAGE_SIZE, REQUEST_TIMEOUT)
            .await
            .map_err(Failure::from)?;
        self.window(&window).await;
        Ok(window)
    }
}

async fn flush(cache: &cache::Cache, pending: &mut HashMap<SessionId, Vec<Event>>) {
    for (session, events) in pending.drain() {
        if let Err(error) = cache.append(session, &events).await {
            tracing::warn!(%session, %error, "Cannot persist live history");
        }
    }
}

async fn write_loop(cache: Option<cache::Cache>, mut writes: mpsc::Receiver<Write>) {
    let mut pending = HashMap::<SessionId, Vec<Event>>::new();
    let mut forgotten = HashSet::new();
    let mut interval = tokio::time::interval(FLUSH_INTERVAL);
    loop {
        let write = tokio::select! {
            write = writes.recv() => write,
            _ = interval.tick() => Some(Write::Flush),
        };
        let Some(cache) = &cache else {
            if write.is_none() {
                return;
            }
            continue;
        };
        match write {
            Some(Write::Event(event)) if !forgotten.contains(&event.session) => {
                let batch = pending.entry(event.session).or_default();
                batch.push(*event);
                if batch.len() >= BATCH_SIZE {
                    flush(cache, &mut pending).await;
                }
            }
            Some(Write::Window(window)) if !forgotten.contains(&window.session.meta.id) => {
                flush(cache, &mut pending).await;
                if let Err(error) = cache.store_window(&window).await {
                    tracing::warn!(%error, "Cannot persist history window");
                }
            }
            Some(Write::Forget(session)) => {
                forgotten.insert(session);
                pending.remove(&session);
                if let Err(error) = cache.remove(session).await {
                    tracing::warn!(%session, %error, "Cannot remove cached history");
                }
            }
            Some(Write::Flush) => flush(cache, &mut pending).await,
            None => {
                flush(cache, &mut pending).await;
                return;
            }
            _ => {}
        }
    }
}
