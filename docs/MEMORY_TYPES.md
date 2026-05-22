# Memory types

memora's defining decision is that not all agent memory should be stored or
treated the same way. We use six typed categories, each with its own
intended use, lifecycle, and merge behaviour.

## The six types

### Episodic

What happened during a session — turns, tool calls, decisions, attempts.
Episodic memory is high-volume and inherently low-importance after the
session ends. It is the natural source for a `memora replay` flight
recorder.

- Lifecycle: typically born `ephemeral`, rarely promoted.
- TTL: short by default; GC takes them quickly.
- Examples:
  - "User asked to refactor `UserService`."
  - "Ran `cargo test`; 3 failures in `auth_test.rs`."
  - "Reverted change to `src/auth/jwt.rs` after test failure."

### Semantic

Stable facts the agent believes about the world or the codebase. These are
the long-lived nuggets you want surviving across sessions and tools.

- Lifecycle: should reach `stable` quickly via promotion.
- TTL: none by default.
- Examples:
  - "Auth uses JWT RS256."
  - "The DB is Postgres 15, hosted on RDS."

### Procedural

Reusable how-to patterns and workflows. Procedural memory captures *how* to
do something correctly in this project.

- Lifecycle: typically `stable` shortly after first verification.
- TTL: none by default.
- Examples:
  - "To deploy staging: `make build && ./scripts/deploy.sh staging`."
  - "Run `cargo fmt --check` before committing."

### Assumption

Beliefs the agent is currently operating on but has *not* verified. The
existence of this category is itself a feature — it lets the agent be
honest about what it is guessing.

- Lifecycle: starts `ephemeral`; either promotes to `semantic`/`stable`
  on confirmation, or expires.
- TTL: shortish by default.
- Examples:
  - "Assuming Redis is the cache layer (not yet confirmed)."
  - "Assuming the user wants TypeScript, not JavaScript."

### Project

Codebase entities, architecture, file structure, naming conventions.
Project memory describes the *artefact* the agent is working on.

- Lifecycle: typically `stable` once written.
- TTL: none by default.
- Examples:
  - "Entry point: `src/main.rs`."
  - "Repository pattern: each domain entity has `<entity>_repo.rs`."

### Preference

User or team preferences about style, tooling, communication. The most
*human-facing* category.

- Lifecycle: usually `stable`.
- TTL: none.
- Examples:
  - "User prefers verbose error messages."
  - "Team uses Conventional Commits."

## Default confidence by source

These are the priors used when `--confidence` is not supplied:

| Source            | Confidence |
| ----------------- | ---------- |
| `code_read`       | 1.00       |
| `test_result`     | 0.90       |
| `manual`          | 0.80       |
| `claude_code` / `cursor` / `cline` / `openhands` | 0.70 |
| `model_inference` | 0.60       |
| `unknown:<name>`  | 0.30       |

## Lifecycle

```
Ephemeral ──promote──▶ Stable ──gc──▶ Deprecated
    │                    │
    └────conflict────────┴──▶ Conflicted
```

Promotion (Phase 2) happens when:

- The same content is re-observed from an independent source.
- A user explicitly runs `memora promote`.
- Confidence crosses a per-project threshold.

Conflicts (Phase 3) happen when two stable nodes describe the same
entity / topic with contradictory content. The CRDT merge engine surfaces
them rather than silently picking a winner.
