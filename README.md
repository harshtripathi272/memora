# memora

> **The memory layer for AI agents — versioned, typed, portable, inspectable.**

[![CI](https://github.com/harshtripathi272/memora/actions/workflows/ci.yml/badge.svg)](https://github.com/harshtripathi272/memora/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Format version](https://img.shields.io/badge/format-v1-informational)](SPEC.md)
[![Made with Rust](https://img.shields.io/badge/made%20with-Rust-orange.svg)](https://www.rust-lang.org/)

memora gives every AI coding agent a real memory: a typed,
version-controlled, provenance-tracked store of what the agent believes
about your project, where each belief came from, and how trustworthy it
is. Commit beliefs like code, branch them before risky experiments, roll
back when the agent goes wrong, replay a session step-by-step, and
export the whole thing to Claude Code, Cursor, Cline, or OpenHands.

In neuroscience, an *engram* is the physical trace a memory leaves in the
brain. **memora** is the engineering equivalent — a durable, queryable,
shareable trace of what your agent has learned.

---

## Why memora

Today every AI agent has goldfish memory:

- A crashed session wipes hours of context.
- Switching from Claude Code to Cursor wipes everything again.
- When the agent quietly forms a wrong belief, there's no way to debug it.
- A whole team's worth of "what we know about this codebase" lives in
  scattered `CLAUDE.md` / `.cursorrules` files that nobody syncs.

memora fixes this by treating agent memory as a first-class artefact:
**typed**, **versioned**, **content-addressed**, **trust-scored**, and
**shareable**.

---

## Install

memora ships as a single static binary with zero runtime dependencies
(SQLite is bundled). Pick whichever path is easiest:

### One-liner (recommended)

**macOS / Linux:**
```bash
curl -fsSL https://raw.githubusercontent.com/harshtripathi272/memora/main/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/harshtripathi272/memora/main/install.ps1 | iex
```

### From a GitHub Release (manual)

Go to [Releases](https://github.com/harshtripathi272/memora/releases/latest),
download the archive for your platform, extract, and put `memora` (or
`memora.exe`) somewhere on your PATH.

### From source

```bash
git clone https://github.com/harshtripathi272/memora.git
cd memora
cargo install --path crates/memora-cli
```

You need a Rust toolchain (1.78+); install via [rustup](https://rustup.rs/).

---

## 60-second demo

```bash
# Start tracking memory in your project
memora init

# Record what the agent learns
memora add --type semantic   --content "Auth uses JWT RS256"        --source code-read --evidence "src/auth/jwt.rs:L42"
memora add --type project    --content "Entry point: src/main.rs"   --source code-read
memora add --type assumption --content "Probably uses Redis"         --source model-inference

# Snapshot it
memora commit -m "first beliefs"

# Confirm the guess
memora promote --type assumption
memora commit -m "confirmed Redis after reading docker-compose.yml"

# See the belief change
memora diff HEAD~1 HEAD --semantic
# → ~ [assumption] ephemeral → stable: Probably uses Redis

# Branch before a risky experiment, then merge it back
memora branch experiment/new-auth
memora switch experiment/new-auth
# ... agent explores ...
memora commit -m "explored OAuth2"
memora switch main
memora merge experiment/new-auth

# Record and replay an entire agent session
memora session start --source claude_code
# ... do work ...
memora session end
memora replay --step

# Export for any tool
memora export --to claude-code      # → CLAUDE.md
memora export --to cursor           # → .cursorrules
memora export --to openai-assistant # → JSON instructions

# Share with the team
memora remote add origin /shared/team-memora
memora push origin
```

---

## The typed memory model

Not all memory is equal. memora splits memory into six typed categories,
each with different storage rules, expiry behaviour, and merge
semantics. This is the core differentiator vs. "just use a vector DB".

| Type        | What it captures                            | Example                                              |
| ----------- | ------------------------------------------- | ---------------------------------------------------- |
| Episodic    | What happened during a session              | "Tried refactor of `UserService`, reverted on tests" |
| Semantic    | Stable facts about the world or codebase    | "Auth module uses JWT RS256 (confirmed)"             |
| Procedural  | Reusable workflows / how-to patterns        | "To deploy: `make build && ./scripts/deploy.sh`"     |
| Assumption  | Unverified beliefs the agent operates on    | "Assuming Redis is the cache — not yet confirmed"    |
| Project     | Codebase entities, architecture, conventions| "Entry point: `src/main.rs`, repository pattern"     |
| Preference  | User / team preferences                     | "User prefers verbose error messages, Rust > Python" |

---

## Provenance and trust

Every memory node carries a full provenance record: where it came from,
what evidence backs it, and a `confidence` float. Defaults by source:

| Source            | Default confidence |
| ----------------- | ------------------ |
| `code-read`       | 1.00               |
| `test-result`     | 0.90               |
| `manual`          | 0.80               |
| `claude-code` / `cursor` / `cline` / `openhands` | 0.70 |
| `model-inference` | 0.60               |
| `unknown:<name>`  | 0.30               |

Lifecycle:

```
Ephemeral ──promote──▶ Stable ──gc──▶ Deprecated ──gc──▶ deleted
    │                    │
    └────conflict────────┴──▶ Conflicted (awaits human resolution)
```

---

## Commands

| Command                                   | Description                                              |
| ----------------------------------------- | -------------------------------------------------------- |
| `memora init [DIR]`                       | Create a `.memora/` store in the current (or given) dir. |
| `memora add --type ... --content ...`     | Add a typed memory node.                                 |
| `memora commit -m "..."`                  | Snapshot the current working set into a commit.          |
| `memora status`                           | Show what has changed since HEAD.                        |
| `memora log [--oneline] [-n N]`           | Print commit history.                                    |
| `memora branch [NAME]`                    | List or create branches.                                 |
| `memora switch NAME`                      | Move HEAD to an existing branch (working set follows).   |
| `memora rollback --to <commit>`           | Reset HEAD to a previous commit (auto-checkpoint first). |
| `memora promote --id <NODE> \| --type <KIND> \| --all-confirmed [T]` | Promote ephemeral nodes to stable. |
| `memora diff [FROM] [TO] [--working] [--semantic]` | Show belief changes between two revisions.        |
| `memora merge BRANCH [--strategy auto\|ours\|theirs] [--no-ff] [--no-commit] [--dry-run]` | Three-way merge another branch into HEAD. |
| `memora session start \| end \| current \| list` | Bracket a tool's run so events are recorded for replay. |
| `memora replay [--session ID] [--step]`   | Walk through a recorded session's event stream.          |
| `memora export --to <FORMAT> [...]`       | Render the working set to `claude-code`, `cursor`, `cline`, `openai-assistant`, or `json`. |
| `memora gc [--threshold T] [--aggressive] [--dry-run]` | Two-phase importance-scored garbage collection.   |
| `memora remote add NAME URL \| list \| remove NAME` | Manage filesystem remotes.                          |
| `memora push REMOTE [BRANCH]`             | Push a branch to a configured remote (fast-forward only).|
| `memora pull REMOTE [BRANCH]`             | Copy remote commits and update `refs/remotes/<remote>/<branch>`. |

This is the full v0.1 surface. Future versions will add `import`,
embeddings-driven semantic merge, and a real network transport.

---

## How is this different from a vector DB?

A vector DB stores everything as opaque embeddings, with no notion of:

- **Type** — every chunk is a blob; you can't say "this is a stable fact"
  vs. "this is an unverified assumption".
- **Provenance** — there is no first-class `source` / `evidence` /
  `confidence`.
- **Lifecycle** — there is no ephemeral / stable / deprecated transition.
- **History** — you can't ask "what did the agent believe last Tuesday?"
  or "show me the commit where it learned this".
- **Branching / merge** — you can't fork memory before a risky run, and
  you can't merge two agents' contradictory beliefs with conflict resolution.

memora is complementary to vector search, not a replacement. It is the
*system of record* for agent memory; embeddings are an implementation
detail of one query path.

---

## Repository layout

```
memora/
├── Cargo.toml                   # workspace manifest
├── crates/
│   ├── memora-core/             # library: types, store, repository
│   └── memora-cli/              # binary: memora
├── docs/
│   ├── ARCHITECTURE.md
│   └── MEMORY_TYPES.md
├── SPEC.md                      # the .memora/ on-disk format
├── CHANGELOG.md
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
├── README.md
└── LICENSE
```

---

## Status

memora is **pre-1.0** (currently `0.1.x`). The on-disk format version
is `1`. Until v1.0 the format may change in non-backwards-compatible
ways; the `format_version` field in `.memora/config` will be bumped
whenever this happens.

A test suite of ~70 cases (core library + CLI integration) gates every
change. Clippy runs in strict mode (`-D warnings`).

## Contributing

Issues, PRs, and discussions all welcome. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for setup, style, and the test
expectations. By participating you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md).

## License

MIT — see [LICENSE](LICENSE).
