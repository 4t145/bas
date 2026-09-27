use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::Mutex,
};

use basr::model::{Cursor, Event, SessionId, SessionMeta};
use futures::future::BoxFuture;

use super::{
    Error, Page, ResumeData, Storage, StoredSession, check_page, validate_commit, validate_create,
};

struct Entry {
    session: StoredSession,
    events: VecDeque<Event>,
    evicted_through: Cursor,
}

/// In-process storage retaining at most `capacity` events per session.
pub struct Memory {
    capacity: NonZeroUsize,
    sessions: Mutex<HashMap<SessionId, Entry>>,
}

impl Memory {
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl Storage for Memory {
    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            validate_create(&meta)?;
            let mut sessions = self.sessions.lock().expect("storage lock is not poisoned");
            if sessions.contains_key(&meta.id) {
                return Err(Error::AlreadyExists(meta.id));
            }
            sessions.insert(
                meta.id,
                Entry {
                    session: StoredSession { meta, resume },
                    events: VecDeque::new(),
                    evicted_through: Cursor::START,
                },
            );
            Ok(())
        })
    }

    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get_mut(&meta.id).ok_or(Error::NotFound(meta.id))?;
            validate_commit(&entry.session.meta, &meta, events)?;
            for event in events {
                if entry.events.len() == self.capacity.get() {
                    entry.evicted_through = entry
                        .events
                        .pop_front()
                        .expect("a full event window is nonempty")
                        .cursor;
                }
                entry.events.push_back(event.clone());
            }
            entry.session.meta = meta;
            Ok(())
        })
    }

    fn save_resume(
        &self,
        id: SessionId,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.sessions
                .lock()
                .expect("storage lock is not poisoned")
                .get_mut(&id)
                .ok_or(Error::NotFound(id))?
                .session
                .resume = resume;
            Ok(())
        })
    }

    fn load(&self, id: SessionId) -> BoxFuture<'_, Result<Option<StoredSession>, Error>> {
        Box::pin(async move {
            Ok(self
                .sessions
                .lock()
                .expect("storage lock is not poisoned")
                .get(&id)
                .map(|entry| entry.session.clone()))
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<SessionMeta>, Error>> {
        Box::pin(async move {
            let mut metas: Vec<_> = self
                .sessions
                .lock()
                .expect("storage lock is not poisoned")
                .values()
                .map(|entry| entry.session.meta.clone())
                .collect();
            metas.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
            Ok(metas)
        })
    }

    fn page(
        &self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Page, Error>> {
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get(&id).ok_or(Error::NotFound(id))?;
            check_page(after, limit, entry.session.meta.revision)?;
            if let Some(requested) = after
                && requested < entry.evicted_through
            {
                return Err(Error::Expired {
                    requested,
                    evicted_through: entry.evicted_through,
                });
            }
            let mut iter = entry
                .events
                .iter()
                .filter(|event| after.is_none_or(|cursor| event.cursor > cursor));
            let events: Vec<_> = iter.by_ref().take(limit).cloned().collect();
            let next = iter
                .next()
                .and_then(|_| events.last().map(|event| event.cursor));
            Ok(Page { events, next })
        })
    }

    fn tail(&self, id: SessionId, count: usize) -> BoxFuture<'_, Result<Vec<Event>, Error>> {
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get(&id).ok_or(Error::NotFound(id))?;
            Ok(entry
                .events
                .iter()
                .skip(entry.events.len().saturating_sub(count))
                .cloned()
                .collect())
        })
    }

    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, Error>> {
        Box::pin(async move {
            Ok(self
                .sessions
                .lock()
                .expect("storage lock is not poisoned")
                .remove(&id)
                .is_some())
        })
    }

    fn is_persistent(&self) -> bool {
        false
    }
}
