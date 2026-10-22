-- Raw events remain the append journal; completed turns can be packed without
-- changing their logical cursors. Existing history is not rewritten at startup.
CREATE TABLE history_blocks (
    session_id TEXT NOT NULL,
    first_cursor INTEGER NOT NULL,
    last_cursor INTEGER NOT NULL,
    turn_id TEXT NOT NULL,
    format INTEGER NOT NULL,
    codec INTEGER NOT NULL,
    raw_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL,
    floor INTEGER NOT NULL,
    PRIMARY KEY (session_id, first_cursor)
);
-- #[toasty::breakpoint]
CREATE INDEX history_blocks_end ON history_blocks(session_id, last_cursor, first_cursor);
-- #[toasty::breakpoint]
CREATE TABLE history_block_context (
    session_id TEXT NOT NULL,
    block_start INTEGER NOT NULL,
    context_key TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    context_start INTEGER NOT NULL,
    PRIMARY KEY (session_id, block_start, context_key)
);
-- #[toasty::breakpoint]
CREATE INDEX history_block_context_latest ON history_block_context(session_id, context_key, cursor);
-- #[toasty::breakpoint]
PRAGMA user_version = 6;
