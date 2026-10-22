//! Old layouts are inspected only for migration; runtime uses three history tables.
use super::*;

type Column = (&'static str, &'static str, i64, i64);

async fn verify(db: &toasty::Db, tables: &[(&str, &[Column])]) -> Result<(), Error> {
    for (table, expected) in tables {
        let rows = sql::query(format!("PRAGMA table_info({table})"))
            .exec(&mut db.clone())
            .await?;
        let mut actual = std::collections::HashMap::new();
        for row in rows {
            let fields = columns(row)?;
            let name = String::try_from(fields[1].clone())
                .map_err(|_| Error::Corrupt("history column"))?;
            let kind = String::try_from(fields[2].clone())
                .map_err(|_| Error::Corrupt("history column type"))?;
            let required = i64::try_from(fields[3].clone())
                .map_err(|_| Error::Corrupt("history nullability"))?;
            let key =
                i64::try_from(fields[5].clone()).map_err(|_| Error::Corrupt("history key"))?;
            actual.insert(name, (kind.to_ascii_uppercase(), required, key));
        }
        if actual.len() != expected.len()
            || expected.iter().any(|(name, kind, required, key)| {
                actual.get(*name) != Some(&(kind.to_string(), *required, *key))
            })
        {
            return Err(Error::Corrupt("incompatible history schema"));
        }
    }
    Ok(())
}

pub(super) async fn verify_history(db: &toasty::Db) -> Result<(), Error> {
    verify_history_version(db, 8).await
}

pub(super) async fn verify_history_version(db: &toasty::Db, version: i64) -> Result<(), Error> {
    let mut segments = vec![
        ("session_id", "TEXT", 1, 1),
        ("first_cursor", "INTEGER", 1, 2),
        ("last_cursor", "INTEGER", 1, 0),
        ("page_start", "INTEGER", 1, 0),
        ("kind", "TEXT", 1, 0),
        ("turn_id", "TEXT", 0, 0),
        ("terminal_cursor", "INTEGER", 0, 0),
        ("sealed", "INTEGER", 1, 0),
        ("packed", "INTEGER", 1, 0),
        ("floor", "INTEGER", 1, 0),
        ("context", "BLOB", 1, 0),
    ];
    if version >= 8 {
        segments.extend([
            ("timestamp_millis", "INTEGER", 1, 0),
            ("preview", "TEXT", 1, 0),
        ]);
    }
    verify(
        db,
        &[
            (
                "history_chunks",
                &[
                    ("session_id", "TEXT", 1, 1),
                    ("first_cursor", "INTEGER", 1, 2),
                    ("last_cursor", "INTEGER", 1, 0),
                    ("segment_start", "INTEGER", 0, 0),
                    ("format", "INTEGER", 1, 0),
                    ("codec", "INTEGER", 1, 0),
                    ("raw_bytes", "INTEGER", 1, 0),
                    ("payload", "BLOB", 1, 0),
                ],
            ),
            ("history_segments", &segments),
            (
                "history_state",
                &[
                    ("session_id", "TEXT", 1, 1),
                    ("cursor", "INTEGER", 1, 0),
                    ("state", "BLOB", 1, 0),
                ],
            ),
        ],
    )
    .await
}

pub(super) async fn verify_legacy_blocks(db: &toasty::Db) -> Result<(), Error> {
    verify(
        db,
        &[
            (
                "history_blocks",
                &[
                    ("session_id", "TEXT", 1, 1),
                    ("first_cursor", "INTEGER", 1, 2),
                    ("last_cursor", "INTEGER", 1, 0),
                    ("turn_id", "TEXT", 1, 0),
                    ("format", "INTEGER", 1, 0),
                    ("codec", "INTEGER", 1, 0),
                    ("raw_bytes", "INTEGER", 1, 0),
                    ("payload", "BLOB", 1, 0),
                    ("floor", "INTEGER", 1, 0),
                ],
            ),
            (
                "history_block_context",
                &[
                    ("session_id", "TEXT", 1, 1),
                    ("block_start", "INTEGER", 1, 2),
                    ("context_key", "TEXT", 1, 3),
                    ("cursor", "INTEGER", 1, 0),
                    ("context_start", "INTEGER", 1, 0),
                ],
            ),
        ],
    )
    .await
}
