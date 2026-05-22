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
- `create_branch`, `list_branches`, `switch_branch`
- `rollback_to` (auto-checkpoints before moving HEAD)
- `promote` (ephemeral → stable, by id / kind / confidence threshold)
- `diff` (graph diff between two revspecs, with optional semantic summary)
- `resolve_revision` (`HEAD`, `HEAD~N`, branch names, hex prefixes)

The diff engine compares two `node_versions` snapshots from the SQLite
store and produces a `DiffReport` with `added` / `removed` / `modified`
buckets. `ModifiedNode` carries a list of typed `NodeChange` deltas
(`Status`, `Content`, `Confidence`, `Source`, `Evidence`) so callers can
render high-level summaries without parsing strings.

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

The roadmap in `README.md` calls out Phase 3 → Phase 5. Notable gaps:

- CRDT merge (`memora merge`).
- Replay (`memora replay`, session event recording).
- Export / import adapters (`memora export --to=claude-code`, etc.).
- GC + remote sync.

The internal types and SQLite tables already make room for these (see
`sessions` / `session_events`, `MemoryStatus::Conflicted`, the per-commit
`node_versions` snapshot table); we'll layer the workflows on top in
subsequent commits.
