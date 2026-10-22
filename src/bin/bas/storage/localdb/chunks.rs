//! One logical event stream over small active chunks and packed history chunks.
use super::*;
use std::ops::RangeInclusive;
use toasty::Transaction;

#[path = "chunks/codec.rs"]
mod codec;
#[path = "chunks/packing.rs"]
mod packing;
#[cfg(test)]
#[path = "chunks/tests.rs"]
mod tests;

const MAX_BLOCK_BYTES: usize = 4 * 1024 * 1024;
const FORMAT: i64 = 1;
const READ_BATCH: i64 = 256;

pub(super) struct Chunk {
    pub first: Cursor,
    pub last: Cursor,
    pub events: Vec<Event>,
}

/// Fetch a byte-bounded batch without decoding unrequested chunks. Oversized
/// individual raw events are admitted alone, just as before chunk packing.
pub(super) async fn rows(
    tx: &mut Transaction<'_>,
    id: SessionId,
    range: RangeInclusive<Cursor>,
    limit: usize,
) -> Result<Vec<Value>, Error> {
    if range.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let headers = sql::query("SELECT first_cursor, last_cursor, raw_bytes, length(payload) FROM history_chunks WHERE session_id = ?1 AND last_cursor >= ?2 AND first_cursor <= ?3 ORDER BY first_cursor LIMIT ?4")
        .bind(id.to_string()).bind(raw(*range.start())?).bind(raw(*range.end())?).bind(READ_BATCH)
        .exec(&mut *tx).await?;
    let mut first = None;
    let mut last = Cursor::START;
    let mut bytes = 0_u64;
    let mut count = 0_u64;
    for row in headers {
        let mut fields = columns(row)?;
        let start = revision(field(&mut fields)?)?;
        let end = revision(field(&mut fields)?)?;
        let decoded = revision(field(&mut fields)?)?.raw();
        let encoded = revision(field(&mut fields)?)?.raw();
        if start == Cursor::START || start > end || decoded == 0 || encoded == 0 {
            return Err(Error::Corrupt("invalid history chunk header"));
        }
        let cost = decoded.max(encoded);
        if first.is_some() && bytes.saturating_add(cost) > MAX_BLOCK_BYTES as u64 {
            break;
        }
        first.get_or_insert(start);
        last = start;
        bytes = bytes.saturating_add(cost);
        count =
            count.saturating_add(end.min(*range.end()).raw() - start.max(*range.start()).raw() + 1);
        if count >= limit as u64 || bytes >= MAX_BLOCK_BYTES as u64 {
            break;
        }
    }
    let Some(first) = first else {
        return Ok(Vec::new());
    };
    sql::query("SELECT first_cursor, last_cursor, format, codec, raw_bytes, payload FROM history_chunks WHERE session_id = ?1 AND first_cursor BETWEEN ?2 AND ?3 ORDER BY first_cursor")
        .bind(id.to_string()).bind(raw(first)?).bind(raw(last)?).exec(&mut *tx).await.map_err(Into::into)
}

pub(super) async fn decode(
    tx: &mut Transaction<'_>,
    id: SessionId,
    latest: Cursor,
    row: Value,
) -> Result<Chunk, Error> {
    let mut fields = columns(row)?;
    let first = revision(field(&mut fields)?)?;
    let last = revision(field(&mut fields)?)?;
    let format: i64 = field(&mut fields)?;
    let encoding: i64 = field(&mut fields)?;
    let size: i64 = field(&mut fields)?;
    let payload: Vec<u8> = field(&mut fields)?;
    if first == Cursor::START || first > last || last > latest || format != FORMAT {
        return Err(Error::Corrupt("invalid history chunk range or format"));
    }
    if (size > MAX_BLOCK_BYTES as i64 || payload.len() > MAX_BLOCK_BYTES)
        && (first != last || encoding != 0)
    {
        return Err(Error::Corrupt("oversized history chunk"));
    }
    if first != last {
        let overlaps = sql::query("SELECT 1 FROM history_chunks WHERE session_id = ?1 AND first_cursor != ?2 AND first_cursor <= ?3 AND last_cursor >= ?2 LIMIT 1")
            .bind(id.to_string()).bind(raw(first)?).bind(raw(last)?).exec(&mut *tx).await?;
        if !overlaps.is_empty() {
            return Err(Error::Corrupt("overlapping history chunks"));
        }
    }
    let events = codec::decode(encoding, size, &payload)?;
    if events.len() as u64 != last.raw() - first.raw() + 1 {
        return Err(Error::Corrupt("history chunk count mismatch"));
    }
    for (offset, event) in events.iter().enumerate() {
        if event.session != id || event.cursor.raw() != first.raw() + offset as u64 {
            return Err(Error::Corrupt("history chunk event mismatch"));
        }
    }
    Ok(Chunk {
        first,
        last,
        events,
    })
}

pub(super) async fn read(
    tx: &mut Transaction<'_>,
    id: SessionId,
    latest: Cursor,
    range: RangeInclusive<Cursor>,
    limit: usize,
) -> Result<Vec<Event>, Error> {
    let mut cursor = *range.start();
    let end = *range.end();
    if end > latest {
        return Err(Error::Corrupt("history range ahead of snapshot"));
    }
    let mut events = Vec::new();
    while cursor <= end && events.len() < limit {
        let batch = rows(tx, id, cursor..=end, limit - events.len()).await?;
        if batch.is_empty() {
            return Err(Error::Corrupt("missing history chunks"));
        }
        for row in batch {
            let chunk = decode(tx, id, latest, row).await?;
            if chunk.first > cursor || chunk.last < cursor {
                return Err(Error::Corrupt("noncontiguous history chunks"));
            }
            events.extend(
                chunk
                    .events
                    .into_iter()
                    .filter(|event| event.cursor >= cursor && event.cursor <= end)
                    .take(limit - events.len()),
            );
            cursor = chunk.last.next();
            if cursor > end || events.len() == limit {
                break;
            }
        }
    }
    Ok(events)
}
