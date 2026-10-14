CREATE TABLE IF NOT EXISTS sessions (
 id TEXT PRIMARY KEY, metadata BLOB NOT NULL, updated_at INTEGER NOT NULL,
 revision INTEGER NOT NULL, resume_version INTEGER, resume_payload BLOB,
 CHECK ((resume_version IS NULL) = (resume_payload IS NULL))
);

-- #[toasty::breakpoint]
CREATE INDEX IF NOT EXISTS sessions_recent ON sessions(updated_at DESC, id DESC);

-- #[toasty::breakpoint]
CREATE TABLE IF NOT EXISTS events (
 session_id TEXT NOT NULL, cursor INTEGER NOT NULL, payload BLOB NOT NULL,
 PRIMARY KEY(session_id, cursor)
);

-- #[toasty::breakpoint]
PRAGMA user_version = 1;
