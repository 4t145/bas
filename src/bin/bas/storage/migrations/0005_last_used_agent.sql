CREATE TABLE agent_selection (
    key TEXT NOT NULL PRIMARY KEY,
    agent TEXT NOT NULL
);
-- #[toasty::breakpoint]
PRAGMA user_version = 5;
