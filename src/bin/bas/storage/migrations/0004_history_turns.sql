-- Only derived state is invalidated; authoritative history is never rewritten.
DELETE FROM history_index;
-- #[toasty::breakpoint]
DELETE FROM history_dependencies;
-- #[toasty::breakpoint]
DELETE FROM history_context_keys;
-- #[toasty::breakpoint]
CREATE TABLE history_turns (
    session_id TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    PRIMARY KEY (session_id, cursor)
);
-- #[toasty::breakpoint]
PRAGMA user_version = 4;
