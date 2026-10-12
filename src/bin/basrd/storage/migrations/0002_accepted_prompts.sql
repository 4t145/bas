CREATE TABLE IF NOT EXISTS accepted_prompts (
 session_id TEXT NOT NULL, turn_id TEXT NOT NULL,
 cursor INTEGER NOT NULL CHECK (cursor > 0),
 fingerprint BLOB NOT NULL CHECK (typeof(fingerprint) = 'blob' AND length(fingerprint) = 32),
 PRIMARY KEY(session_id, turn_id)
);

-- #[toasty::breakpoint]
PRAGMA user_version = 2;
