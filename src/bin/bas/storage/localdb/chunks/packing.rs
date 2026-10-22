use super::*;

impl SqliteStore {
    /// Compaction is maintenance after commit, never part of request acceptance.
    pub(in crate::storage::localdb) async fn pack_ready(
        &self,
        id: SessionId,
        changed: RangeInclusive<Cursor>,
    ) -> Result<(), Error> {
        let rows = sql::query("SELECT first_cursor FROM history_segments WHERE session_id = ?1 AND sealed = 1 AND packed = 0 AND last_cursor BETWEEN ?2 AND ?3 ORDER BY first_cursor")
            .bind(id.to_string()).bind(raw(*changed.start())?).bind(raw(*changed.end())?)
            .exec(&mut self.db.clone()).await?;
        for row in rows {
            self.pack_segment(id, revision(field(&mut columns(row)?)?)?)
                .await?;
        }
        Ok(())
    }

    pub(in crate::storage::localdb) async fn pack_segment(
        &self,
        id: SessionId,
        start: Cursor,
    ) -> Result<(), Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let row = sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(id.to_string()).exec(&mut tx).await?.into_iter().next().ok_or(Error::NotFound(id))?;
        let latest = decode_session(row)?.meta.revision;
        let row = sql::query("SELECT last_cursor, sealed, packed FROM history_segments WHERE session_id = ?1 AND first_cursor = ?2")
            .bind(id.to_string()).bind(raw(start)?).exec(&mut tx).await?.into_iter().next()
            .ok_or(Error::Corrupt("missing segment"))?;
        let mut fields = columns(row)?;
        let through = revision(field(&mut fields)?)?;
        let sealed: i64 = field(&mut fields)?;
        let packed: i64 = field(&mut fields)?;
        if sealed != 1 || packed == 1 {
            return Ok(());
        }
        if start > through || through > latest {
            return Err(Error::Corrupt("invalid segment range"));
        }
        let mut after = start.raw() - 1;
        let mut events = Vec::new();
        let mut bytes = 2;
        while after < through.raw() {
            // Length-only scan: leave huge single events untouched without loading them.
            let headers = sql::query("SELECT first_cursor, last_cursor, raw_bytes FROM history_chunks WHERE session_id = ?1 AND first_cursor > ?2 AND first_cursor <= ?3 ORDER BY first_cursor LIMIT ?4")
                .bind(id.to_string()).bind(after as i64).bind(raw(through)?).bind(READ_BATCH)
                .exec(&mut tx).await?;
            if headers.is_empty() {
                return Err(Error::Corrupt("missing chunks during packing"));
            }
            for row in headers {
                let mut fields = columns(row)?;
                let first = revision(field(&mut fields)?)?;
                let last = revision(field(&mut fields)?)?;
                let size: i64 = field(&mut fields)?;
                if first.raw() != after + 1 || last < first || last > through {
                    return Err(Error::Corrupt("noncontiguous segment chunks"));
                }
                after = last.raw();
                if size > MAX_BLOCK_BYTES as i64 {
                    seal(&mut tx, id, start, &events).await?;
                    events.clear();
                    bytes = 2;
                    continue;
                }
                let row = sql::query("SELECT first_cursor, last_cursor, format, codec, raw_bytes, payload FROM history_chunks WHERE session_id = ?1 AND first_cursor = ?2")
                    .bind(id.to_string()).bind(raw(first)?).exec(&mut tx).await?.into_iter().next()
                    .ok_or(Error::Corrupt("chunk disappeared"))?;
                let chunk = decode(&mut tx, id, latest, row).await?;
                // Never split an existing chunk: replacement must cover whole source
                // rows, including after a legacy migration or an index rebuild.
                let size = serde_json::to_vec(&chunk.events)?.len();
                if bytes + size - 1 > MAX_BLOCK_BYTES {
                    seal(&mut tx, id, start, &events).await?;
                    events.clear();
                    bytes = 2;
                }
                if size > MAX_BLOCK_BYTES {
                    // A newer canonical encoding can grow; retain the original row.
                    continue;
                }
                bytes = if events.is_empty() {
                    size
                } else {
                    bytes + size - 1
                };
                events.extend(chunk.events);
            }
        }
        seal(&mut tx, id, start, &events).await?;
        sql::statement(
            "UPDATE history_segments SET packed = 1 WHERE session_id = ?1 AND first_cursor = ?2",
        )
        .bind(id.to_string())
        .bind(raw(start)?)
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn seal(
    tx: &mut Transaction<'_>,
    id: SessionId,
    segment: Cursor,
    events: &[Event],
) -> Result<(), Error> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let last = events.last().expect("nonempty chunk");
    let encoded = codec::encode(events)?;
    // Replacement and deletion share a transaction; any failure restores the old chunks.
    sql::statement("DELETE FROM history_chunks WHERE session_id = ?1 AND first_cursor >= ?2 AND last_cursor <= ?3")
        .bind(id.to_string()).bind(raw(first.cursor)?).bind(raw(last.cursor)?).exec(&mut *tx).await?;
    sql::statement("INSERT INTO history_chunks(session_id, first_cursor, last_cursor, segment_start, format, codec, raw_bytes, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
        .bind(id.to_string()).bind(raw(first.cursor)?).bind(raw(last.cursor)?).bind(raw(segment)?)
        .bind(FORMAT).bind(encoded.codec).bind(encoded.raw_bytes as i64).bind(encoded.payload)
        .exec(&mut *tx).await?;
    Ok(())
}
