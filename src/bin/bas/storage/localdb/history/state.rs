use super::*;
use bas::model::{
    EventKind,
    history::{DependencyIndex, append_prompt_preview},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ContextRef {
    pub first: Cursor,
    pub last: Cursor,
}
pub(super) type Context = BTreeMap<String, ContextRef>;

#[derive(Default, Serialize, Deserialize)]
struct State {
    dependencies: DependencyIndex,
    context: Context,
}

struct Segment {
    first: Cursor,
    last: Cursor,
    page_start: Cursor,
    kind: String,
    turn: Option<TurnId>,
    terminal: Option<Cursor>,
    sealed: bool,
    floor: Cursor,
    context: Context,
    timestamp_millis: i64,
    preview: String,
}

impl Segment {
    async fn save(&self, tx: &mut toasty::Transaction<'_>, id: SessionId) -> Result<(), Error> {
        sql::statement("INSERT INTO history_segments(session_id, first_cursor, last_cursor, page_start, kind, turn_id, terminal_cursor, sealed, packed, floor, context, timestamp_millis, preview) VALUES (?1, ?2, ?3, ?4, ?5, NULLIF(?6, ''), ?7, ?8, 0, ?9, ?10, ?11, ?12) ON CONFLICT(session_id, first_cursor) DO UPDATE SET last_cursor = excluded.last_cursor, terminal_cursor = excluded.terminal_cursor, sealed = excluded.sealed, floor = excluded.floor, preview = excluded.preview")
            .bind(id.to_string()).bind(raw(self.first)?).bind(raw(self.last)?).bind(raw(self.page_start)?)
            .bind(self.kind.clone()).bind(self.turn.map(|turn| turn.to_string()).unwrap_or_default())
            .bind_typed(self.terminal.map(raw).transpose()?, toasty::schema::db::Type::Integer(8))
            .bind(i64::from(self.sealed))
            .bind(raw(self.floor)?).bind(serde_json::to_vec(&self.context)?)
            .bind(self.timestamp_millis).bind(self.preview.clone())
            .exec(&mut *tx).await?;
        Ok(())
    }
}

pub(in crate::storage::localdb) async fn index(
    tx: &mut toasty::Transaction<'_>,
    id: SessionId,
    latest: Cursor,
) -> Result<(), Error> {
    let row = sql::query("SELECT cursor FROM history_state WHERE session_id = ?1")
        .bind(id.to_string())
        .exec(&mut *tx)
        .await?
        .into_iter()
        .next();
    let (mut cursor, mut state) = if let Some(row) = row {
        let mut fields = columns(row)?;
        let cursor = revision(field(&mut fields)?)?;
        if cursor == latest {
            return Ok(());
        }
        // Up-to-date readers need only the high-water mark, not the potentially
        // session-sized identity state used by incremental indexing.
        let row = sql::query("SELECT state FROM history_state WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut *tx)
            .await?
            .into_iter()
            .next()
            .ok_or(Error::Corrupt("missing history state"))?;
        let bytes: Vec<u8> = field(&mut columns(row)?)?;
        (cursor, serde_json::from_slice::<State>(&bytes)?)
    } else {
        // All catalog data is derived. A rebuild never rewrites chunk payloads.
        sql::statement("DELETE FROM history_segments WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut *tx)
            .await?;
        sql::statement("UPDATE history_chunks SET segment_start = NULL WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut *tx)
            .await?;
        (Cursor::START, State::default())
    };
    if cursor > latest {
        return Err(Error::Corrupt("history state ahead of snapshot"));
    }
    let row = sql::query("SELECT first_cursor, last_cursor, page_start, kind, turn_id, terminal_cursor, sealed, floor, context, timestamp_millis, preview FROM history_segments WHERE session_id = ?1 ORDER BY first_cursor DESC LIMIT 1")
        .bind(id.to_string()).exec(&mut *tx).await?.into_iter().next();
    let mut segment = row
        .map(|row| {
            let mut fields = columns(row)?;
            let first = revision(field(&mut fields)?)?;
            let last = revision(field(&mut fields)?)?;
            if last != cursor {
                return Err(Error::Corrupt("segment does not match history state"));
            }
            let page_start = revision(field(&mut fields)?)?;
            let kind = field(&mut fields)?;
            let turn = optional_string(&mut fields)?
                .map(|id| {
                    id.parse::<TurnId>()
                        .map_err(|_| Error::Corrupt("segment turn id"))
                })
                .transpose()?;
            let terminal = optional_cursor(&mut fields)?;
            let sealed = boolean(field(&mut fields)?)?;
            let floor = revision(field(&mut fields)?)?;
            let context: Vec<u8> = field(&mut fields)?;
            Ok(Segment {
                first,
                last,
                page_start,
                kind,
                turn,
                terminal,
                sealed,
                floor,
                context: serde_json::from_slice(&context)?,
                timestamp_millis: field(&mut fields)?,
                preview: field(&mut fields)?,
            })
        })
        .transpose()?;
    if cursor != Cursor::START && segment.is_none() {
        return Err(Error::Corrupt("history state has no segment"));
    }
    while cursor < latest {
        let batch = chunks::rows(tx, id, cursor.next()..=latest, usize::MAX).await?;
        if batch.is_empty() {
            return Err(Error::Corrupt("missing history during indexing"));
        }
        for row in batch {
            let chunk = chunks::decode(tx, id, latest, row).await?;
            if chunk.first != cursor.next() {
                return Err(Error::Corrupt("history state cuts a chunk"));
            }
            let mut owner = None;
            for event in &chunk.events {
                let dependency = state.dependencies.observe(event);
                let page_start = dependency.segment_start;
                if segment
                    .as_ref()
                    .is_none_or(|s| s.sealed || s.page_start != page_start)
                {
                    if let Some(previous) = &mut segment {
                        previous.sealed = true;
                        previous.save(tx, id).await?;
                    }
                    let turn = if let EventKind::TurnStarted { turn } = event.kind {
                        Some(turn)
                    } else {
                        None
                    };
                    let kind = if turn.is_some() {
                        "turn"
                    } else if dependency.starts_prompt {
                        "imported"
                    } else if event.cursor == Cursor::from_raw(1) {
                        "initialization"
                    } else {
                        "control"
                    };
                    segment = Some(Segment {
                        first: event.cursor,
                        last: event.cursor,
                        page_start,
                        kind: kind.into(),
                        turn,
                        terminal: None,
                        sealed: false,
                        floor: event.cursor,
                        context: state.context.clone(),
                        timestamp_millis: i64::try_from(event.at.millis()).unwrap_or(i64::MAX),
                        preview: String::new(),
                    });
                }
                let current = segment.as_mut().expect("segment established");
                if owner.is_some_and(|owner| owner != current.first) {
                    return Err(Error::Corrupt("chunk crosses history segments"));
                }
                owner = Some(current.first);
                current.last = event.cursor;
                current.floor = current.floor.min(dependency.floor);
                if matches!(
                    event.kind,
                    EventKind::Acp(bas::acp::SessionUpdate::UserMessageChunk(_))
                ) {
                    // Identity updates can address a prompt in an earlier logical
                    // page, including one whose physical segment is already sealed.
                    let page = if dependency.floor >= current.first {
                        current.page_start
                    } else {
                        let row = sql::query("SELECT page_start FROM history_segments WHERE session_id = ?1 AND first_cursor <= ?2 AND last_cursor >= ?2 ORDER BY first_cursor DESC LIMIT 1")
                            .bind(id.to_string()).bind(raw(dependency.floor)?).exec(&mut *tx).await?
                            .into_iter().next().ok_or(Error::Corrupt("missing prompt dependency segment"))?;
                        revision(field(&mut columns(row)?)?)?
                    };
                    if page == current.first {
                        append_prompt_preview(&mut current.preview, event);
                    } else {
                        let row = sql::query("SELECT preview FROM history_segments WHERE session_id = ?1 AND first_cursor = ?2 AND kind IN ('turn', 'imported')")
                            .bind(id.to_string()).bind(raw(page)?).exec(&mut *tx).await?.into_iter().next();
                        if let Some(row) = row {
                            let mut preview: String = field(&mut columns(row)?)?;
                            append_prompt_preview(&mut preview, event);
                            sql::statement("UPDATE history_segments SET preview = ?1 WHERE session_id = ?2 AND first_cursor = ?3")
                                .bind(preview).bind(id.to_string()).bind(raw(page)?).exec(&mut *tx).await?;
                        }
                    }
                }
                if let Some(key) = dependency.context_key {
                    if let Some(first) = dependency.context_start {
                        state.context.insert(
                            key,
                            ContextRef {
                                first,
                                last: event.cursor,
                            },
                        );
                    } else {
                        state.context.remove(&key);
                    }
                }
                if let EventKind::TurnFinished { turn, .. }
                | EventKind::TurnInterrupted { turn, .. } = &event.kind
                    && current.turn == Some(*turn)
                {
                    // Keep unrelated terminal events in history without treating
                    // them as completion of this segment's explicit turn.
                    current.terminal = Some(event.cursor);
                    current.sealed = true;
                }
                cursor = event.cursor;
            }
            let current = segment.as_ref().expect("nonempty chunk");
            current.save(tx, id).await?;
            sql::statement("UPDATE history_chunks SET segment_start = ?1 WHERE session_id = ?2 AND first_cursor = ?3")
                .bind(raw(owner.expect("chunk owner"))?).bind(id.to_string()).bind(raw(chunk.first)?)
                .exec(&mut *tx).await?;
        }
    }
    sql::statement("INSERT INTO history_state(session_id, cursor, state) VALUES (?1, ?2, ?3) ON CONFLICT(session_id) DO UPDATE SET cursor = excluded.cursor, state = excluded.state")
        .bind(id.to_string()).bind(raw(cursor)?).bind(serde_json::to_vec(&state)?).exec(&mut *tx).await?;
    Ok(())
}

fn optional_string(fields: &mut Vec<Value>) -> Result<Option<String>, Error> {
    if matches!(fields.first(), Some(Value::Null)) {
        fields.remove(0);
        Ok(None)
    } else {
        field(fields).map(Some)
    }
}
fn optional_cursor(fields: &mut Vec<Value>) -> Result<Option<Cursor>, Error> {
    if matches!(fields.first(), Some(Value::Null)) {
        fields.remove(0);
        Ok(None)
    } else {
        revision(field(fields)?).map(Some)
    }
}
fn boolean(value: i64) -> Result<bool, Error> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::Corrupt("invalid segment flag")),
    }
}
