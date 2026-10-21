CREATE TABLE history_index (
 session_id TEXT PRIMARY KEY NOT NULL,
 cursor INTEGER NOT NULL,
 state BLOB NOT NULL
);
-- #[toasty::breakpoint]
CREATE TABLE history_dependencies (
 session_id TEXT NOT NULL,
 cursor INTEGER NOT NULL,
 floor INTEGER NOT NULL,
 context_key TEXT NOT NULL,
 context_start INTEGER NOT NULL,
 PRIMARY KEY(session_id, cursor)
);
-- #[toasty::breakpoint]
CREATE INDEX history_context ON history_dependencies(session_id, context_key, cursor);
-- #[toasty::breakpoint]
CREATE TABLE history_context_keys (
 session_id TEXT NOT NULL,
 context_key TEXT NOT NULL,
 PRIMARY KEY(session_id, context_key)
);
-- #[toasty::breakpoint]
PRAGMA user_version = 3;
