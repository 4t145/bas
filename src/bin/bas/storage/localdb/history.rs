//! Logical turn windows over the segment catalog, independent of chunk packing.
use super::*;
use crate::storage::{Location, Window, check_location, check_window};
use bas::service::{HistoryOverview, HistoryOverviewEntry};

#[path = "history/state.rs"]
mod state;
pub(super) use state::index;

#[derive(Clone, Copy)]
enum Selection {
    Before(Option<Cursor>, usize),
    At(Cursor),
}

impl SqliteStore {
    pub(super) async fn overview(&self, id: SessionId) -> Result<HistoryOverview, Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let row = sql::query("SELECT revision FROM sessions WHERE id = ?1")
            .bind(id.to_string())
            .exec(&mut tx)
            .await?
            .into_iter()
            .next()
            .ok_or(Error::NotFound(id))?;
        let cursor = revision(field(&mut columns(row)?)?)?;
        index(&mut tx, id, cursor).await?;
        let rows = sql::query("SELECT first_cursor, timestamp_millis, preview FROM history_segments WHERE session_id = ?1 AND first_cursor = page_start AND kind IN ('turn', 'imported') ORDER BY first_cursor")
            .bind(id.to_string()).exec(&mut tx).await?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let mut fields = columns(row)?;
            entries.push(HistoryOverviewEntry {
                cursor: revision(field(&mut fields)?)?.raw(),
                timestamp_millis: field(&mut fields)?,
                preview: field(&mut fields)?,
            });
        }
        tx.commit().await?;
        Ok(HistoryOverview {
            session: id,
            cursor: cursor.raw(),
            entries,
        })
    }

    pub(super) async fn window_at(&self, id: SessionId, cursor: Cursor) -> Result<Location, Error> {
        self.locate(id, Selection::At(cursor)).await
    }

    pub(super) async fn window(
        &self,
        id: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> Result<Window, Error> {
        Ok(self
            .locate(id, Selection::Before(before, limit))
            .await?
            .window)
    }

    async fn locate(&self, id: SessionId, selection: Selection) -> Result<Location, Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let row = sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(id.to_string()).exec(&mut tx).await?.into_iter().next().ok_or(Error::NotFound(id))?;
        let meta = decode_session(row)?.meta;
        match selection {
            Selection::Before(before, limit) => check_window(before, limit, meta.revision)?,
            Selection::At(cursor) => check_location(cursor, meta.revision)?,
        }
        index(&mut tx, id, meta.revision).await?;
        let (before, limit) = match selection {
            Selection::Before(before, limit) => (before, limit),
            Selection::At(cursor) => {
                let row = sql::query("SELECT page_start FROM history_segments WHERE session_id = ?1 AND first_cursor <= ?2 AND last_cursor >= ?2 ORDER BY first_cursor DESC LIMIT 1")
                    .bind(id.to_string()).bind(raw(cursor)?).exec(&mut tx).await?
                    .into_iter().next().ok_or(Error::Corrupt("missing containing page"))?;
                let start = revision(field(&mut columns(row)?)?)?;
                let row = sql::query("SELECT first_cursor FROM history_segments WHERE session_id = ?1 AND first_cursor = page_start AND first_cursor > ?2 ORDER BY first_cursor LIMIT 1")
                    .bind(id.to_string()).bind(raw(start)?).exec(&mut tx).await?.into_iter().next();
                let after = row
                    .map(|row| revision(field(&mut columns(row)?)?))
                    .transpose()?;
                (after, 1)
            }
        };
        check_window(before, limit, meta.revision)?;
        let after = before.filter(|cursor| *cursor <= meta.revision);
        let end = before.map_or(meta.revision.raw(), |before| before.raw().saturating_sub(1));
        if let Some(before) = before
            && before.raw() > 1
            && before <= meta.revision
        {
            let rows = sql::query("SELECT first_cursor FROM history_segments WHERE session_id = ?1 AND first_cursor = ?2 AND page_start = first_cursor")
                .bind(id.to_string()).bind(raw(before)?).exec(&mut tx).await?;
            if rows.is_empty() {
                return Err(Error::NotTurnBoundary(before));
            }
        }
        let rows = sql::query("SELECT first_cursor FROM history_segments WHERE session_id = ?1 AND first_cursor = page_start AND first_cursor <= ?2 ORDER BY first_cursor DESC LIMIT ?3")
            .bind(id.to_string()).bind(end as i64).bind(i64::try_from(limit).unwrap_or(i64::MAX)).exec(&mut tx).await?;
        let mut start = 1;
        for row in rows {
            start = revision(field(&mut columns(row)?)?)?.raw();
        }
        loop {
            let row = sql::query("SELECT MIN(floor) FROM history_segments WHERE session_id = ?1 AND first_cursor >= ?2 AND last_cursor <= ?3")
                .bind(id.to_string()).bind(start as i64).bind(end as i64).exec(&mut tx).await?
                .into_iter().next().ok_or(Error::Corrupt("missing dependency aggregate"))?;
            let mut row = columns(row)?;
            if matches!(row.first(), Some(Value::Null)) {
                break;
            }
            let floor = revision(field(&mut row)?)?.raw();
            if floor >= start {
                break;
            }
            let row = sql::query("SELECT page_start FROM history_segments WHERE session_id = ?1 AND first_cursor <= ?2 AND last_cursor >= ?2 ORDER BY first_cursor DESC LIMIT 1")
                .bind(id.to_string()).bind(floor as i64).exec(&mut tx).await?
                .into_iter().next().ok_or(Error::Corrupt("missing containing segment"))?;
            let expanded = revision(field(&mut columns(row)?)?)?.raw();
            if expanded == 0 || expanded >= start {
                return Err(Error::Corrupt("invalid segment dependency floor"));
            }
            start = expanded;
        }
        let events = chunks::read(
            &mut tx,
            id,
            meta.revision,
            Cursor::from_raw(start)..=Cursor::from_raw(end),
            usize::MAX,
        )
        .await?;
        if events.len() as u64 != end.saturating_add(1).saturating_sub(start) {
            return Err(Error::Corrupt("noncontiguous window"));
        }
        let mut context_cursors = std::collections::BTreeSet::new();
        if start <= end {
            let row = sql::query(
                "SELECT context FROM history_segments WHERE session_id = ?1 AND first_cursor = ?2",
            )
            .bind(id.to_string())
            .bind(start as i64)
            .exec(&mut tx)
            .await?
            .into_iter()
            .next()
            .ok_or(Error::Corrupt("missing window context"))?;
            let bytes: Vec<u8> = field(&mut columns(row)?)?;
            let context: state::Context = serde_json::from_slice(&bytes)?;
            let has_usage = events.iter().any(|event| {
                matches!(
                    event.kind,
                    bas::model::EventKind::Acp(bas::acp::SessionUpdate::UsageUpdate(_))
                )
            });
            for (key, reference) in context {
                if key == "usage" && has_usage {
                    continue;
                }
                if reference.first == Cursor::START
                    || reference.first > reference.last
                    || reference.last.raw() >= start
                {
                    return Err(Error::Corrupt("invalid segment context reference"));
                }
                context_cursors.insert(reference.first);
                context_cursors.insert(reference.last);
            }
        }
        let mut context = Vec::new();
        for cursor in context_cursors {
            context.extend(chunks::read(&mut tx, id, meta.revision, cursor..=cursor, 1).await?);
        }
        let before = events
            .first()
            .filter(|event| event.cursor.raw() > 1)
            .map(|event| event.cursor);
        tx.commit().await?;
        Ok(Location {
            window: Window {
                cursor: meta.revision,
                meta,
                events,
                context,
                before,
            },
            after,
        })
    }
}
