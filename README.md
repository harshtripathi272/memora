# memora

> **The memory layer for AI agents — versioned, typed, portable, and inspectable.**

memora gives every AI coding agent a real memory: a typed, version-controlled,
provenance-tracked store of what the agent believes, where each belief came
from, and how trustworthy it is. Commit memory like code, branch it before
risky experiments, roll back when the agent goes wrong, and export it to any
tool (Claude Code, Cursor, Cline, OpenHands).

In neuroscience, an *engram* is the physical trace a memory leaves in the
brain. **memora** is the engineering equivalent — a durable, queryable trace
of what your agent has learned.

---

## Why memora

Today every AI agent has goldfish memory:

- A crashed session wipes hours of context.
- Switching from Claude Code to Cursor wipes everything again.
- When the agent quietly forms a wrong belief, there is no way to debug it.

memora fixes this by treating agent memory as a first-class artefact: typed,
versioned, content-addressed, and shareable.

---

## Quick start

```bash
# Install (placeholder — release artefacts coming soon)
cargo install --path crates/memora-cli

# Initialise a memora store in your project
memora init

# Add a typed memory
memora add --type semantic \
           --content "Auth module uses JWT RS256" \
           --source code-read \
           --evidence "src/auth/jwt.rs:L42"

# Snapshot it
memora commit -m "learned auth scheme"

# See history
memora log --oneline

# Branch before a risky experiment, then roll back if it goes wrong
memora branch experiment/refactor-auth
memora switch experiment/refactor-auth
memora rollback --to <commit>
```

---

## The typed memory model

Not all memory is equal. memora splits memory into six typed categories — each
with different storage rules, expiry behaviour, and merge semantics. This is
the core differentiator versus "just use a vector DB".

| Type        | What it captures                                   | Example                                              |
| ----------- | -------------------------------------------------- | ---------------------------------------------------- |
| Episodic    | What happened during a session                     | "Tried refactor of `UserService`, reverted on tests" |
| Semantic    | Stable facts about the world or codebase           | "Auth module uses JWT RS256 (confirmed)"             |
| Procedural  | Reusable workflows / how-to patterns               | "To deploy: `make build && ./scripts/deploy.sh`"     |
| Assumption  | Unverified beliefs the agent is operating on       | "Assuming Redis is the cache — not yet confirmed"    |
| Project     | Codebase entities, architecture, conventions       | "Entry point: `src/main.rs`, repository pattern"     |
| Preference  | User / team preferences                            | "User prefers verbose error messages, Rust > Python" |

---

## Provenance and trust

Every memory node carries a full provenance record: where it came from, what
evidence backs it, and a `confidence` float. The default trust ranking by
source is:

| Source          | Default confidence |
| --------------- | ------------------ |
| `code-read`     | 1.00               |
| `test-result`   | 0.90               |
| `manual`        | 0.80               |
| `claude-code` / `cursor` / `cline` / `openhands` | 0.70 |
| `model-inference` | 0.60             |
| `unknown:<name>` | 0.30              |

Every node also carries a lifecycle status:

```
Ephemeral ──promote──▶ Stable ──gc──▶ Deprecated
    │                    │
    └────conflict────────┴──▶ Conflicted
```

---

## Commands (v0.1)

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

Future phases add `import`, `gc`, `push`, `pull`. See `SPEC.md` for the
full roadmap.

---

## How is this different from a vector DB?

A vector DB stores everything as opaque embeddings, with no notion of:

- **Type** — every chunk is a blob; you can't say "this is a stable fact" vs.
  "this is an unverified assumption".
- **Provenance** — there is no first-class `source` / `evidence` / `confidence`.
- **Lifecycle** — there is no ephemeral / stable / deprecated transition.
- **History** — you can't ask "what did the agent believe last Tuesday?" or
  "show me the commit where it learned this".
- **Branching** — there is no way to fork memory before a risky run.

memora is complementary: it is the *system of record* for agent memory.
Embeddings are an implementation detail of one query path; type, provenance
and history are the primary model.

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
├── README.md
└── LICENSE
```

---

## Status

memora is pre-alpha (v0.1.x). The on-disk format version is `1`. Until v1.0
the format may change in non-backwards-compatible ways; we will bump the
format version field whenever this happens.

---

## License

MIT — see [LICENSE](LICENSE).
