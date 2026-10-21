use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::Mutex,
};

use bas::model::history::{Dependency, DependencyIndex};
use bas::model::{Cursor, Event, SessionId, SessionMeta, TurnId};
use futures::future::BoxFuture;

use super::{
    Error, Page, PromptAcceptance, ResumeData, Storage, StoredSession, Window, check_page,
    check_window, validate_acceptance, validate_commit, validate_create,
};

struct Entry {
    session: StoredSession,
    events: VecDeque<Event>,
    evicted_through: Cursor,
    acceptances: HashMap<TurnId, PromptAcceptance>,
    index: DependencyIndex,
    dependencies: VecDeque<Dependency>,
    context: HashMap<String, Context>,
    context_overflow: bool,
}

#[derive(Clone)]
struct Context {
    first: Cursor,
    last: Cursor,
}

/// In-process storage retaining at most `capacity` events per session.
/// Acceptance receipts survive event eviction until session deletion: memory grows
/// by one hash/receipt and map entry per accepted prompt, never by its payload.
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
                    acceptances: HashMap::new(),
                    index: DependencyIndex::default(),
                    dependencies: VecDeque::new(),
                    context: HashMap::new(),
                    context_overflow: false,
                },
            );
            Ok(())
        })
    }

    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get_mut(&meta.id).ok_or(Error::NotFound(meta.id))?;
            if let Some(acceptance) = acceptance {
                if entry.acceptances.contains_key(&acceptance.receipt.turn) {
                    return Err(Error::SubmissionExists(acceptance.receipt.turn));
                }
                validate_acceptance(events, acceptance)?;
            }
            validate_commit(&entry.session.meta, &meta, events)?;
            for event in events {
                let dependency = entry.index.observe(event);
                if entry.events.len() == self.capacity.get() {
                    let dependency = entry
                        .dependencies
                        .pop_front()
                        .expect("parallel dependency queue");
                    if let Some(key) = dependency.context_key {
                        match dependency.context_start {
                            Some(first)
                                if entry.context.len() < self.capacity.get()
                                    || entry.context.contains_key(&key) =>
                            {
                                entry.context.insert(
                                    key,
                                    Context {
                                        first,
                                        last: entry.events.front().expect("full queue").cursor,
                                    },
                                );
                            }
                            Some(_) => entry.context_overflow = true,
                            None => {
                                entry.context.remove(&key);
                            }
                        }
                    }
                    entry.evicted_through = entry
                        .events
                        .pop_front()
                        .expect("a full event window is nonempty")
                        .cursor;
                    entry.index.evict_through(entry.evicted_through);
                }
                entry.events.push_back(event.clone());
                entry.dependencies.push_back(dependency);
            }
            entry.session.meta = meta;
            if let Some(acceptance) = acceptance {
                entry
                    .acceptances
                    .insert(acceptance.receipt.turn, acceptance);
            }
            Ok(())
        })
    }

    fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> BoxFuture<'_, Result<Option<PromptAcceptance>, Error>> {
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get(&session).ok_or(Error::NotFound(session))?;
            Ok(entry.acceptances.get(&turn).copied())
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

    fn window(
        &self,
        id: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Window, Error>> {
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("storage lock is not poisoned");
            let entry = sessions.get(&id).ok_or(Error::NotFound(id))?;
            let meta = entry.session.meta.clone();
            check_window(before, limit, meta.revision)?;
            if before.is_some_and(|cursor| cursor.raw() <= 1) {
                return Ok(Window {
                    cursor: meta.revision,
                    meta,
                    events: vec![],
                    context: vec![],
                    before: None,
                });
            }
            if entry.context_overflow {
                return Err(Error::Expired {
                    requested: Cursor::START,
                    evicted_through: entry.evicted_through,
                });
            }
            let end = entry
                .events
                .partition_point(|event| before.is_none_or(|b| event.cursor < b));
            if end == 0 && entry.evicted_through != Cursor::START {
                return Err(Error::Expired {
                    requested: before.unwrap_or(Cursor::START),
                    evicted_through: entry.evicted_through,
                });
            }
            if let Some(before) = before
                && before <= meta.revision
                && entry
                    .dependencies
                    .get(end)
                    .is_none_or(|d| d.segment_start != before)
            {
                return Err(Error::NotTurnBoundary(before));
            }
            let mut start = end;
            for _ in 0..limit {
                if start == 0 {
                    break;
                }
                let boundary = entry.dependencies[start - 1].segment_start;
                if boundary <= entry.evicted_through {
                    return Err(Error::Expired {
                        requested: boundary,
                        evicted_through: entry.evicted_through,
                    });
                }
                start = entry
                    .events
                    .partition_point(|event| event.cursor < boundary);
            }
            loop {
                let floor = entry.dependencies.range(start..end).map(|d| d.floor).min();
                let Some(floor) = floor else { break };
                if floor <= entry.evicted_through {
                    return Err(Error::Expired {
                        requested: floor,
                        evicted_through: entry.evicted_through,
                    });
                }
                let containing = entry.events.partition_point(|event| event.cursor < floor);
                let boundary = entry.dependencies[containing].segment_start;
                if boundary <= entry.evicted_through {
                    return Err(Error::Expired {
                        requested: boundary,
                        evicted_through: entry.evicted_through,
                    });
                }
                let expanded = entry
                    .events
                    .partition_point(|event| event.cursor < boundary);
                if expanded >= start {
                    break;
                }
                start = expanded;
            }
            let mut context = entry.context.clone();
            for (event, dependency) in entry.events.iter().zip(&entry.dependencies).take(start) {
                if let Some(key) = &dependency.context_key {
                    if let Some(first) = dependency.context_start {
                        context.insert(
                            key.clone(),
                            Context {
                                first,
                                last: event.cursor,
                            },
                        );
                    } else {
                        context.remove(key);
                    }
                }
            }
            if entry
                .dependencies
                .range(start..end)
                .any(|dependency| dependency.context_key.as_deref() == Some("usage"))
            {
                context.remove("usage");
            }
            let mut context_cursors: Vec<_> = context
                .into_values()
                .flat_map(|context| [context.first, context.last])
                .collect();
            context_cursors.sort_unstable();
            context_cursors.dedup();
            let mut context = Vec::new();
            for cursor in context_cursors {
                let event = entry
                    .events
                    .iter()
                    .find(|event| event.cursor == cursor)
                    .ok_or(Error::Expired {
                        requested: cursor,
                        evicted_through: entry.evicted_through,
                    })?;
                context.push(event.clone());
            }
            let events: Vec<_> = entry.events.range(start..end).cloned().collect();
            let before = events
                .first()
                .filter(|event| event.cursor.raw() > 1)
                .map(|event| event.cursor);
            Ok(Window {
                cursor: meta.revision,
                meta,
                events,
                context,
                before,
            })
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
