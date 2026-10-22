-- Prompt labels are derived from authoritative chunks, never a second event log.
ALTER TABLE history_segments ADD COLUMN timestamp_millis INTEGER NOT NULL DEFAULT 0;
-- #[toasty::breakpoint]
ALTER TABLE history_segments ADD COLUMN preview TEXT NOT NULL DEFAULT '';
-- #[toasty::breakpoint]
DELETE FROM history_state;
-- #[toasty::breakpoint]
DELETE FROM history_segments;
-- #[toasty::breakpoint]
UPDATE history_chunks SET segment_start = NULL;
-- #[toasty::breakpoint]
PRAGMA user_version = 8;
