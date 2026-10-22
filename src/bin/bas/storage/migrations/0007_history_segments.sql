-- One physical history representation. Existing events become single-event
-- arrays; existing v6 blocks already use that format. No history is discarded.
CREATE TABLE history_chunks (
    session_id TEXT NOT NULL,
    first_cursor INTEGER NOT NULL,
    last_cursor INTEGER NOT NULL,
    segment_start INTEGER,
    format INTEGER NOT NULL,
    codec INTEGER NOT NULL,
    raw_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (session_id, first_cursor)
);
-- #[toasty::breakpoint]
CREATE INDEX history_chunks_end ON history_chunks(session_id, last_cursor, first_cursor);
-- #[toasty::breakpoint]
CREATE INDEX history_chunks_segment ON history_chunks(session_id, segment_start, first_cursor);
-- #[toasty::breakpoint]
CREATE TABLE history_segments (
    session_id TEXT NOT NULL,
    first_cursor INTEGER NOT NULL,
    last_cursor INTEGER NOT NULL,
    page_start INTEGER NOT NULL,
    kind TEXT NOT NULL,
    turn_id TEXT,
    terminal_cursor INTEGER,
    sealed INTEGER NOT NULL,
    packed INTEGER NOT NULL,
    floor INTEGER NOT NULL,
    context BLOB NOT NULL,
    PRIMARY KEY (session_id, first_cursor)
);
-- #[toasty::breakpoint]
CREATE INDEX history_segments_pages ON history_segments(session_id, page_start, first_cursor);
-- #[toasty::breakpoint]
CREATE INDEX history_segments_end ON history_segments(session_id, last_cursor, first_cursor);
-- #[toasty::breakpoint]
CREATE TABLE history_state (
    session_id TEXT PRIMARY KEY NOT NULL,
    cursor INTEGER NOT NULL,
    state BLOB NOT NULL
);
-- #[toasty::breakpoint]
INSERT INTO history_chunks(session_id, first_cursor, last_cursor, segment_start, format, codec, raw_bytes, payload)
SELECT session_id, cursor, cursor, NULL, 1, 0, length(payload) + 2,
       CAST('[' || CAST(payload AS TEXT) || ']' AS BLOB)
FROM events;
-- #[toasty::breakpoint]
INSERT INTO history_chunks(session_id, first_cursor, last_cursor, segment_start, format, codec, raw_bytes, payload)
SELECT session_id, first_cursor, last_cursor, NULL, format, codec, raw_bytes, payload
FROM history_blocks;
-- #[toasty::breakpoint]
DROP TABLE history_block_context;
-- #[toasty::breakpoint]
DROP TABLE history_blocks;
-- #[toasty::breakpoint]
DROP TABLE history_dependencies;
-- #[toasty::breakpoint]
DROP TABLE history_context_keys;
-- #[toasty::breakpoint]
DROP TABLE history_index;
-- #[toasty::breakpoint]
DROP TABLE history_turns;
-- #[toasty::breakpoint]
DROP TABLE events;
-- #[toasty::breakpoint]
PRAGMA user_version = 7;
