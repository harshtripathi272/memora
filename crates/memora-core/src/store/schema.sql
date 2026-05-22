-- memora SQLite schema (format version 1).
--
-- Designed to be inspectable: every table maps directly to a concept in
-- the Rust API. Foreign keys are ON, journal mode is WAL (set by Store).

CREATE TABLE IF NOT EXISTS nodes (
    id              TEXT    PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN
                            ('episodic','semantic','procedural','assumption','project','preference')),
    content         TEXT    NOT NULL,
    confidence      REAL    NOT NULL DEFAULT 1.0,
    status          TEXT    NOT NULL DEFAULT 'ephemeral' CHECK (status IN
                            ('ephemeral','stable','deprecated','conflicted')),
    source          TEXT    NOT NULL,
    evidence        TEXT,
    tags_json       TEXT    NOT NULL DEFAULT '[]',
    related_to_json TEXT    NOT NULL DEFAULT '[]',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    last_accessed   INTEGER NOT NULL,
    access_count    INTEGER NOT NULL DEFAULT 0,
    expires_at      INTEGER
);

CREATE INDEX IF NOT EXISTS idx_nodes_kind   ON nodes (kind);
CREATE INDEX IF NOT EXISTS idx_nodes_status ON nodes (status);
CREATE INDEX IF NOT EXISTS idx_nodes_updated ON nodes (updated_at);

CREATE TABLE IF NOT EXISTS commits (
    id          TEXT    PRIMARY KEY,
    parent_id   TEXT    REFERENCES commits(id),
    message     TEXT    NOT NULL,
    author      TEXT    NOT NULL,
    timestamp   INTEGER NOT NULL,
    tree_id     TEXT    NOT NULL,
    stats_json  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_commits_parent ON commits (parent_id);
CREATE INDEX IF NOT EXISTS idx_commits_ts     ON commits (timestamp);

-- Membership table linking commits to the node ids that made up their tree.
CREATE TABLE IF NOT EXISTS commit_nodes (
    commit_id TEXT NOT NULL REFERENCES commits(id) ON DELETE CASCADE,
    node_id   TEXT NOT NULL,
    PRIMARY KEY (commit_id, node_id)
);

CREATE INDEX IF NOT EXISTS idx_commit_nodes_node ON commit_nodes (node_id);

-- Replay infrastructure. Sessions and their event streams are written here
-- so future `memora replay` can step through context evolution.
CREATE TABLE IF NOT EXISTS sessions (
    id           TEXT    PRIMARY KEY,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER,
    source       TEXT    NOT NULL,
    event_count  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS session_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    timestamp  INTEGER NOT NULL,
    event_type TEXT    NOT NULL,
    data_json  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events (session_id);
