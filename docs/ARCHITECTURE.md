# Architecture

This document is a developer-facing tour of the memora codebase. It is not
a user guide — see `README.md` and `SPEC.md` for those.

## Workspace layout

```
memora/
├── Cargo.toml               # workspace manifest
├── crates/
│   ├── memora-core/         # library — pure types + storage
│   └── memora-cli/          # binary — `memora` CLI on top of the core
└── docs/
```

The split is deliberate: `memora-core` is what every other tool, SDK, or
service should depend on. The CLI is a thin user interface on top.

## Layered architecture

```
   ┌─────────────────────────────┐
   │  memora-cli (clap, ANSI UI) │
   ├─────────────────────────────┤
   │  Repository (high-level)    │   workflow: init / add / commit / log
   ├─────────────────────────────┤
   │  Store + Refs (mid-level)   │   persistence concerns
   ├─────────────────────────────┤
   │  MemoryNode + MemoryCommit  │   pure data types
   └─────────────────────────────┘
```

Higher layers depend on lower layers, never the other way round.

## Crate: `memora-core`

### `error.rs`
A single `MemoraError` enum and `Result<T>` alias. Every fallible function
returns `Result<T, MemoraError>`. CLI converts to `anyhow::Error` for nice
display.

### `node.rs`
The core typed memory model:
- `MemoryKind` — the six categories.
- `MemoryStatus` — lifecycle states.
- `MemorySource` — provenance.
- `MemoryNode` — full record on disk.
- `NewNode` — builder request used by the store.

### `commit.rs`
- `CommitStats` — change counts.
- `MemoryCommit` — a snapshot of a tree of node ids.
- `tree_id_for` and `commit_id` — deterministic content addressing.

### `hash.rs` / `time.rs`
Tiny helpers — SHA-256 hex digesting and a `Clock` trait so tests can pin
timestamps deterministically.

### `store/`
- `schema.sql` — the SQLite schema, embedded at build time.
- `db.rs` — `Store`: owns the connection, exposes node/commit CRUD plus
  `unstaged_against` for `memora status`.
- `refs.rs` — plain-file ref + HEAD management. Validates branch names.

### `repo.rs`
`Repository` is the workflow facade the CLI talks to. It composes `Refs` +
`Store`, holds a `Clock`, and exposes intent-revealing methods:
- `init`, `open`, `discover`, `open_from`
- `add_node`, `status`, `commit`, `log`
- `create_branch`, `list_branches`, `switch_branch` (rewrites the working
  set to match the target tip; refuses with uncommitted changes)
- `rollback_to` (auto-checkpoints before moving HEAD)
- `promote` (ephemeral → stable, by id / kind / confidence threshold)
- `diff` (graph diff between two revspecs, with optional semantic summary)
- `resolve_revision` (`HEAD`, `HEAD~N`, branch names, hex prefixes)
- `plan_merge` / `merge` (three-way merge with auto / ours / theirs
  strategies; produces fast-forward, merge commit, or surfaced conflicts)
- `start_session` / `end_session` / `record_event` / `session_events`
  / `list_sessions` (the flight recorder)
- `ranked_nodes` / `export` (importance-scored ranking + format adapters)

The diff engine compares two `node_versions` snapshots from the SQLite
store and produces a `DiffReport` with `added` / `removed` / `modified`
buckets. `ModifiedNode` carries a list of typed `NodeChange` deltas
(`Status`, `Content`, `Confidence`, `Source`, `Evidence`) so callers can
render high-level summaries without parsing strings.

### `merge.rs`
Pure three-way merge engine. Walks both parent DAGs to find the merge
base, then for each node id decides:

1. *Unchanged* on both sides → keep.
2. Changed on exactly one side → take that side.
3. Changed on both sides → score by **confidence → source priority →
   status priority → recency**. The winning side is `Auto { ours_won }`.
   Genuine ties are returned as `Conflicted`, written into the working
   set as `MemoryStatus::Conflicted`, and surfaced via the merge commit
   stats.
4. `--strategy=ours` / `--strategy=theirs` skips scoring and forces the
   choice without producing conflicts.

Merge commits store their first parent in `commits.parent_id` (so
first-parent log walks keep working) and additional parents in the
`merge_parents` table.

### `session.rs` and `export.rs`
Two small modules that round out the agent-tooling story.

`session.rs` defines `Session` + `SessionEvent` + `SessionEventKind`.
`Repository` writes a marker file `.memora/sessions/CURRENT` containing
the active session id; while a session is active, `add_node`, `commit`,
`promote`, and `merge` all append a typed event to `session_events`.
`memora replay` walks that event stream with optional `--step` pacing.

`export.rs` is a set of pure renderers (`&[MemoryNode] → String`) for
`CLAUDE.md`, `.cursorrules`, `.clinerules`, OpenAI Assistants JSON, and
raw JSON. The repository pre-ranks nodes using the importance formula
`confidence × 0.4 + recency × 0.3 + access × 0.3` (configurable via
`ImportanceWeights`) and applies an `ExportFilter` (kinds, statuses,
min confidence, top N) before handing the nodes to the renderer.

### `gc.rs` and remote sync (`remote.rs`, `config.rs`)

`gc.rs` implements two-phase importance-scored garbage collection:
the first pass marks low-importance nodes as `Deprecated`, the next
pass physically deletes anything currently `Deprecated` from the live
table. `node_versions` is never touched, so historical commits continue
to carry the full snapshot of every GC'd node — restoring is just a
`memora rollback` away. `--aggressive` collapses the two passes into one.

`remote.rs` treats a remote as another `.memora/`-bearing project on
the filesystem. `Repository::push` and `pull` open the remote's SQLite
alongside ours and copy the missing commits in topological order along
with their companion rows (`commit_nodes`, `node_versions`,
`merge_parents`). Pushes are fast-forward-only: if the remote tip is
not an ancestor of ours, the push is rejected so we never overwrite
remote history. After `pull`, the remote tip is recorded at
`refs/remotes/<remote>/<branch>`, and `resolve_revision("origin/main")`
folds it back into the rest of the system, so `memora merge origin/main`
just works. The single `copy_commits_between` function is the entire
transport boundary — replacing it with a real network protocol later
keeps the public API unchanged.

`config.rs` swaps the hand-written init-time TOML for a typed round-trip
(`Config { core, author, remote }`) so adding and removing remotes
produces a tidy, predictable diff against the on-disk file.

## Crate: `memora-cli`

### `cli.rs`
All `clap` parsing lives here. Subcommands map 1:1 to the implementations
in `commands/`.

### `commands/`
One file per subcommand. Each `run(args)` function:
1. Opens the repository (`Repository::open_from`).
2. Calls a single core method.
3. Prints a friendly summary using `ui::*` helpers.

### `ui.rs`
Centralised printing helpers (timestamps, short ids, error formatter).

## Testing strategy

- **Unit tests** in `memora-core` cover types, hashing, refs, and the
  store's CRUD against a tempdir SQLite file.
- **Repository tests** exercise the full `init → add → commit → log →
  branch → rollback` flow end-to-end via the public API.
- **CLI integration tests** in `crates/memora-cli/tests/cli.rs` invoke
  the actual built binary using `assert_cmd`. These guard the user-visible
  contract (exit codes, stdout, stderr).

`cargo test` runs everything; CI will gate on it.

## What's not built yet

The v0.1 surface is complete. Items the original spec keeps as future
work:

- `memora import` — read existing `CLAUDE.md` / `.cursorrules` files back
  into a memora store. JSON round-trip already works (`memora export
  --to json` is consumable by any deserialiser).
- Real network transport for `push` / `pull`. Today's transport is
  filesystem-only; the row-level `copy_commits_between` boundary is
  intentionally narrow so a future Git-protocol or HTTP transport plugs
  in without changing the public API.
- Semantic-overlap detection during merge. Same-id three-way merge is
  in; "different ids, same fact" needs embeddings.
- Embedded ONNX inference for the semantic diff engine.

The internal types and SQLite tables already make room for these (see
`sessions` / `session_events`, the per-commit `node_versions` snapshot
table, `merge_parents`); we'll layer the workflows on top in subsequent
versions.
