//! Durable payload-free index; only a legacy database's first access scans its history.
use super::*;
use crate::storage::{Window, check_window};
use bas::model::history::DependencyIndex;

pub(super) async fn index(
    tx: &mut toasty::db::Transaction<'_>,
    id: SessionId,
    latest: Cursor,
) -> Result<(), Error> {
    let row = sql::query("SELECT cursor FROM history_index WHERE session_id = ?1")
        .bind(id.to_string())
        .exec(&mut *tx)
        .await?
        .into_iter()
        .next();
    let (mut cursor, mut index) = if let Some(row) = row {
        let mut row = columns(row)?;
        let cursor = revision(field(&mut row)?)?;
        if cursor == latest {
            return Ok(());
        }
        let row = sql::query("SELECT state FROM history_index WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut *tx)
            .await?
            .into_iter()
            .next()
            .ok_or(Error::Corrupt("missing dependency state"))?;
        let mut row = columns(row)?;
        let state: Vec<u8> = field(&mut row)?;
        (cursor, serde_json::from_slice::<DependencyIndex>(&state)?)
    } else {
        (Cursor::START, DependencyIndex::default())
    };
    if cursor == latest {
        return Ok(());
    }
    if cursor > latest {
        return Err(Error::Corrupt("dependency index ahead of history"));
    }
    while cursor < latest {
        let rows = sql::query("SELECT cursor, payload FROM events WHERE session_id = ?1 AND cursor > ?2 AND cursor <= ?3 ORDER BY cursor LIMIT 256")
            .bind(id.to_string()).bind(raw(cursor)?).bind(raw(latest)?).exec(&mut *tx).await?;
        if rows.is_empty() {
            return Err(Error::Corrupt("missing events during dependency indexing"));
        }
        for row in rows {
            let event = decode_event(id, latest, row)?;
            if event.cursor != cursor.next() {
                return Err(Error::Corrupt("noncontiguous history"));
            }
            let dependency = index.observe(&event);
            if dependency.segment_start == event.cursor {
                sql::statement("INSERT INTO history_turns(session_id, cursor) VALUES (?1, ?2)")
                    .bind(id.to_string())
                    .bind(raw(event.cursor)?)
                    .exec(&mut *tx)
                    .await?;
            }
            let key = dependency.context_key.unwrap_or_default();
            sql::statement("INSERT INTO history_dependencies(session_id, cursor, floor, context_key, context_start) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(id.to_string()).bind(raw(event.cursor)?).bind(raw(dependency.floor)?).bind(key.clone())
                .bind(dependency.context_start.map(raw).transpose()?.unwrap_or(0))
                .exec(&mut *tx).await?;
            if !key.is_empty() {
                sql::statement("INSERT OR IGNORE INTO history_context_keys(session_id, context_key) VALUES (?1, ?2)")
                    .bind(id.to_string()).bind(key).exec(&mut *tx).await?;
            }
            cursor = event.cursor;
        }
    }
    sql::statement("INSERT INTO history_index(session_id, cursor, state) VALUES (?1, ?2, ?3) ON CONFLICT(session_id) DO UPDATE SET cursor = excluded.cursor, state = excluded.state")
        .bind(id.to_string()).bind(raw(cursor)?).bind(serde_json::to_vec(&index)?).exec(&mut *tx).await?;
    Ok(())
}

impl SqliteStore {
    pub(super) async fn window(
        &self,
        id: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> Result<Window, Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let row = sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(id.to_string()).exec(&mut tx).await?.into_iter().next().ok_or(Error::NotFound(id))?;
        let meta = decode_session(row)?.meta;
        check_window(before, limit, meta.revision)?;
        index(&mut tx, id, meta.revision).await?;
        let end = before.map_or(meta.revision.raw(), |before| before.raw().saturating_sub(1));
        if let Some(before) = before
            && before.raw() > 1
            && before <= meta.revision
        {
            let rows = sql::query(
                "SELECT cursor FROM history_turns WHERE session_id = ?1 AND cursor = ?2",
            )
            .bind(id.to_string())
            .bind(raw(before)?)
            .exec(&mut tx)
            .await?;
            if rows.is_empty() {
                return Err(Error::NotTurnBoundary(before));
            }
        }
        let rows = sql::query("SELECT cursor FROM history_turns WHERE session_id = ?1 AND cursor <= ?2 ORDER BY cursor DESC LIMIT ?3")
            .bind(id.to_string()).bind(end as i64).bind(i64::try_from(limit).unwrap_or(i64::MAX)).exec(&mut tx).await?;
        let mut start = 1;
        for row in rows {
            start = revision(field(&mut columns(row)?)?)?.raw();
        }
        loop {
            let row = sql::query("SELECT MIN(floor) FROM history_dependencies WHERE session_id = ?1 AND cursor >= ?2 AND cursor <= ?3")
                .bind(id.to_string()).bind(start as i64).bind(end as i64).exec(&mut tx).await?
                .into_iter().next().ok_or(Error::Corrupt("missing dependency aggregate"))?;
            let mut row = columns(row)?;
            if matches!(row.first(), Some(Value::Null)) {
                break;
            }
            let floor: i64 = field(&mut row)?;
            let floor = revision(floor)?.raw();
            if floor >= start {
                break;
            }
            let row = sql::query("SELECT cursor FROM history_turns WHERE session_id = ?1 AND cursor <= ?2 ORDER BY cursor DESC LIMIT 1")
                .bind(id.to_string()).bind(floor as i64).exec(&mut tx).await?
                .into_iter().next().ok_or(Error::Corrupt("missing containing turn"))?;
            start = revision(field(&mut columns(row)?)?)?.raw();
        }
        let rows = sql::query("SELECT cursor, payload FROM events WHERE session_id = ?1 AND cursor >= ?2 AND cursor <= ?3 ORDER BY cursor")
            .bind(id.to_string()).bind(start as i64).bind(end as i64).exec(&mut tx).await?;
        let events = rows
            .into_iter()
            .map(|row| decode_event(id, meta.revision, row))
            .collect::<Result<Vec<_>, _>>()?;
        if events.len() as u64 != end.saturating_add(1).saturating_sub(start) {
            return Err(Error::Corrupt("noncontiguous window"));
        }
        let rows = sql::query("SELECT e.cursor, e.payload FROM history_context_keys k JOIN history_dependencies d ON d.session_id = k.session_id AND d.cursor = (SELECT MAX(p.cursor) FROM history_dependencies p WHERE p.session_id = k.session_id AND p.context_key = k.context_key AND p.cursor < ?2) JOIN events e ON e.session_id = d.session_id AND (e.cursor = d.cursor OR e.cursor = d.context_start) WHERE k.session_id = ?1 AND d.context_start > 0 AND (k.context_key != 'usage' OR NOT EXISTS (SELECT 1 FROM history_dependencies p WHERE p.session_id = k.session_id AND p.context_key = 'usage' AND p.cursor BETWEEN ?2 AND ?3)) ORDER BY e.cursor")
            .bind(id.to_string()).bind(start as i64).bind(end as i64).exec(&mut tx).await?;
        let context = rows
            .into_iter()
            .map(|row| decode_event(id, meta.revision, row))
            .collect::<Result<Vec<_>, _>>()?;
        let before = events
            .first()
            .filter(|event| event.cursor.raw() > 1)
            .map(|event| event.cursor);
        tx.commit().await?;
        Ok(Window {
            cursor: meta.revision,
            meta,
            events,
            context,
            before,
        })
    }
}
