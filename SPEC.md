# memora on-disk format — `.memora/` v1

This document specifies the *on-disk* format memora uses inside a project
directory. The goal is for the format to be:

- **Inspectable.** Plain text wherever it doesn't hurt; SQLite for the bulk
  store. You should be able to `cat` the important parts.
- **Tool-portable.** Any agent or SDK can read and write it without going
  through the `memora` binary, as long as it respects this spec.
- **Versioned.** A `format_version` field allows future evolutions; bumps
  are documented in this file.

Status: **format version 1 (pre-1.0, may change).**

---

## Top-level layout

```
.memora/
├── HEAD                  # symbolic ref or detached commit id
├── config                # TOML, hand-readable
├── memora.db             # SQLite database (the object store)
├── refs/
│   ├── heads/            # one file per branch; contents = commit id
│   └── remotes/          # remote-tracking refs (reserved, unused in v1)
├── objects/              # reserved for future content-addressed blobs
└── sessions/             # reserved for future replay sessions
    └── <session-id>/
        ├── events.jsonl
        └── snapshots/
```

### `HEAD`

One line, no trailing whitespace required:

- Normal case: `ref: refs/heads/<branch>`
- Detached:    `<full-commit-sha256>`

### `config`

TOML. Minimum keys for v1:

```toml
[core]
format_version = 1
default_branch = "main"

[author]
name = "human"
```

Tools may add their own sections (e.g. `[claude_code]`) without breaking
compatibility, as long as `[core]` keys remain valid.

### `refs/heads/<branch>`

Either empty (a branch with no commits yet) or a single line containing the
full commit sha256 the branch points at.

---

## SQLite schema (v1)

```sql
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN
        ('episodic','semantic','procedural','assumption','project','preference')),
    content TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0,
    status TEXT NOT NULL DEFAULT 'ephemeral'
        CHECK (status IN ('ephemeral','stable','deprecated','conflicted')),
    source TEXT NOT NULL,
    evidence TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    related_to_json TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL,
    access_count INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER
);

CREATE TABLE commits (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES commits(id),
    message TEXT NOT NULL,
    author TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    tree_id TEXT NOT NULL,
    stats_json TEXT NOT NULL
);

CREATE TABLE commit_nodes (
    commit_id TEXT NOT NULL REFERENCES commits(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    PRIMARY KEY (commit_id, node_id)
);

CREATE TABLE node_versions (
    commit_id TEXT NOT NULL REFERENCES commits(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    evidence TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    related_to_json TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    expires_at INTEGER,
    PRIMARY KEY (commit_id, node_id)
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    source TEXT NOT NULL,
    event_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    timestamp INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    data_json TEXT NOT NULL
);
```

Indexes are `idx_nodes_kind`, `idx_nodes_status`, `idx_nodes_updated`,
`idx_commits_parent`, `idx_commits_ts`, `idx_commit_nodes_node`,
`idx_node_versions_node`, `idx_session_events_session`. They are advisory:
any tool may rebuild them.

`PRAGMA foreign_keys = ON` and `PRAGMA journal_mode = WAL` are required for
correctness and concurrency.

---

## Content addressing

- **Node id**: lowercase hex SHA-256 of
  `"v1\nkind:<kind>\nsource:<source-as-string>\nts:<created_at>\ncontent:<content>"`.
- **Tree id**: lowercase hex SHA-256 of the node ids in the tree, sorted
  ascending and joined with `\n`.
- **Commit id**: lowercase hex SHA-256 of
  `"v1\nparent:<parent-or-empty>\ntree:<tree>\nauthor:<author>\nts:<ts>\nmsg:<msg>"`.

These are the canonical formulas. Implementations MUST produce identical ids
for identical inputs.

---

## Source string canonicalisation

| Variant                | On-disk string         |
| ---------------------- | ---------------------- |
| `ClaudeCode`           | `claude_code`          |
| `Cursor`               | `cursor`               |
| `Cline`                | `cline`                |
| `OpenHands`            | `openhands`            |
| `CodeRead`             | `code_read`            |
| `TestResult`           | `test_result`          |
| `ModelInference`       | `model_inference`      |
| `Manual`               | `manual`               |
| `Unknown(name)`        | `unknown:<name>`       |

Readers should accept both `claude_code` and `claude-code` for friendliness;
writers MUST emit the canonical underscored form.

---

## Format evolution

Any non-backwards-compatible change to this document increments `format_version`.
Tools that see a higher `format_version` than they support SHOULD refuse to
write to the store and present a clear upgrade hint.
