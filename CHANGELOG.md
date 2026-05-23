# Changelog

All notable changes to memora are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-05-23

The first public release. Implements the full v0.1 surface from `SPEC.md`.

### Added

- **Core types.** Six typed memory categories (`episodic`, `semantic`,
  `procedural`, `assumption`, `project`, `preference`), four lifecycle
  statuses (`ephemeral`, `stable`, `deprecated`, `conflicted`), and a
  provenance-aware source enum with default trust scores.
- **Snapshot primitives.** `memora init`, `add`, `commit`, `status`,
  `log`, `branch`, `switch`, `rollback`. Branch switching rewrites the
  working set and refuses with uncommitted changes.
- **Lifecycle.** `memora promote --id | --type | --all-confirmed [T]`
  flips ephemeral nodes to stable. Commit stats track promotions and
  conflicts.
- **Diff.** `memora diff [FROM] [TO] [--working] [--semantic]` produces
  belief-level deltas (status flips, content edits, confidence jumps,
  source changes) plus a natural-language summary.
- **Three-way merge.** `memora merge BRANCH [--strategy auto|ours|theirs]
  [--no-ff] [--no-commit] [--dry-run]`. BFS merge-base across the parent
  DAG, score-based auto-resolution (confidence → source priority →
  status priority → recency), genuine ties surfaced as `Conflicted`.
- **Sessions and replay.** `memora session start | end | current | list`
  brackets a tool's run; `add` / `commit` / `promote` / `merge`
  auto-emit typed events. `memora replay [--session ID] [--step]` walks
  the recorded event stream.
- **Export.** `memora export --to <FORMAT>` for `claude-code` (CLAUDE.md),
  `cursor` (.cursorrules), `cline` (.clinerules), `openai-assistant`
  (JSON), and `json` (lossless). Importance-scored ranking with
  configurable `kind`, `status`, `min-confidence`, and `top` filters.
- **Garbage collection.** `memora gc [--threshold T] [--aggressive]
  [--dry-run]` two-phase mark/sweep. `node_versions` are preserved so
  GC is reversible via rollback.
- **Filesystem remotes.** `memora remote add | list | remove`,
  `memora push | pull`. Fast-forward-only push safety, remote-tracking
  refs at `refs/remotes/<remote>/<branch>`, resolvable as `origin/main`
  in any revspec.

### Notes

- Single-binary CLI (~3.6 MB on Windows). Ships SQLite bundled; no
  external services or runtime dependencies.
- Format version 1; any non-backwards-compatible change will bump it.

[Unreleased]: https://github.com/harshtripathi272/memora/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/harshtripathi272/memora/releases/tag/v0.1.0
