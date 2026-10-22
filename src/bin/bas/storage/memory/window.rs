use super::*;

impl Entry {
    pub(super) fn window(&self, before: Option<Cursor>, limit: usize) -> Result<Window, Error> {
        let meta = self.session.meta.clone();
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
        if self.context_overflow {
            return Err(Error::Expired {
                requested: Cursor::START,
                evicted_through: self.evicted_through,
            });
        }
        let end = self
            .events
            .partition_point(|event| before.is_none_or(|b| event.cursor < b));
        if end == 0 && self.evicted_through != Cursor::START {
            return Err(Error::Expired {
                requested: before.unwrap_or(Cursor::START),
                evicted_through: self.evicted_through,
            });
        }
        if let Some(before) = before
            && before <= meta.revision
            && self
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
            let boundary = self.dependencies[start - 1].segment_start;
            if boundary <= self.evicted_through {
                return Err(Error::Expired {
                    requested: boundary,
                    evicted_through: self.evicted_through,
                });
            }
            start = self.events.partition_point(|event| event.cursor < boundary);
        }
        loop {
            let floor = self.dependencies.range(start..end).map(|d| d.floor).min();
            let Some(floor) = floor else { break };
            if floor <= self.evicted_through {
                return Err(Error::Expired {
                    requested: floor,
                    evicted_through: self.evicted_through,
                });
            }
            let containing = self.events.partition_point(|event| event.cursor < floor);
            let boundary = self.dependencies[containing].segment_start;
            if boundary <= self.evicted_through {
                return Err(Error::Expired {
                    requested: boundary,
                    evicted_through: self.evicted_through,
                });
            }
            let expanded = self.events.partition_point(|event| event.cursor < boundary);
            if expanded >= start {
                break;
            }
            start = expanded;
        }
        let mut context = self.context.clone();
        for (event, dependency) in self.events.iter().zip(&self.dependencies).take(start) {
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
        if self
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
            let event = self
                .events
                .iter()
                .find(|event| event.cursor == cursor)
                .ok_or(Error::Expired {
                    requested: cursor,
                    evicted_through: self.evicted_through,
                })?;
            context.push(event.clone());
        }
        let events: Vec<_> = self.events.range(start..end).cloned().collect();
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
    }
}
