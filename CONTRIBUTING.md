# Contributing to memora

Thanks for considering a contribution. memora is early-stage, so the most
useful contributions right now are bug reports, format-spec feedback, and
small focused PRs. Big feature work is welcome but please open an issue
first to align on direction before writing code.

## Getting set up

You need a recent stable Rust toolchain (1.78+). [Install via
rustup](https://rustup.rs/), then:

```bash
git clone https://github.com/<your-fork>/memora.git
cd memora
cargo build
cargo test
```

`cargo test` runs the full suite (~70 tests). Everything must pass on
Linux, macOS, and Windows; CI gates on it.

## Code style

- `cargo fmt` before committing (the repo uses default rustfmt).
- `cargo clippy --all-targets -- -D warnings` must be clean.
- Public items get rustdoc comments. The crate compiles with
  `#![warn(missing_docs)]`, so missing docs surface as warnings.
- Prefer small, focused commits with descriptive messages. Conventional
  Commits (`feat:`, `fix:`, `docs:`) are appreciated but not required.

## Tests

- New behaviour gets a test. The existing tests in
  `crates/memora-core/src/repo.rs` and `crates/memora-cli/tests/cli.rs`
  are good templates.
- Use `tempfile::tempdir()` for filesystem isolation; never touch the
  user's real `.memora/`.
- Use the `StepClock` pattern from `repo.rs` tests when you need
  deterministic timestamps.

## On-disk format changes

The `.memora/` layout is documented in `SPEC.md`. Any non-backwards-
compatible change must:

1. Bump `FORMAT_VERSION` in `crates/memora-core/src/lib.rs`.
2. Update `SPEC.md`.
3. Add a migration note to `CHANGELOG.md`.

## Questions

Open a [Discussion](https://github.com/harshtripathi272/memora/discussions) or
file an issue. Any question is fine.
