//! End-to-end CLI smoke tests.
//!
//! Runs the actual `memora` binary against a fresh temp directory to make
//! sure init / add / commit / log all work together.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn memora() -> Command {
    let mut cmd = Command::cargo_bin("memora").expect("binary should build");
    // Disable colour so we can parse output reliably.
    cmd.env("NO_COLOR", "1");
    cmd
}

/// Strip ANSI CSI escape sequences. We don't pull in a regex crate just
/// for this — the small state machine below is enough.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC. Consume the next char (typically '[') and skip until a
            // letter, which terminates the CSI sequence.
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next();
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

#[test]
fn init_then_add_then_commit_then_log() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();

    memora()
        .arg("init")
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialised"));

    memora()
        .args([
            "add",
            "--type",
            "semantic",
            "--content",
            "Auth module uses JWT RS256",
            "--source",
            "code-read",
            "--evidence",
            "src/auth/jwt.rs:L42",
            "--tag",
            "auth",
        ])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[semantic]"));

    memora()
        .args(["commit", "-m", "first memory"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("first memory"));

    memora()
        .args(["log", "--oneline"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("first memory"));

    memora()
        .arg("status")
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to commit"));
}

#[test]
fn double_init_fails() {
    let tmp = tempdir().unwrap();
    memora().arg("init").current_dir(tmp.path()).assert().success();
    memora()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn rollback_moves_head_back() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();

    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "v1", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "c1"])
        .current_dir(path)
        .assert()
        .success();

    // Capture the first commit short id from `log --oneline`.
    let out = memora()
        .args(["log", "--oneline"])
        .current_dir(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    let cleaned = strip_ansi(&stdout);
    let first_short = cleaned.split_whitespace().next().unwrap().to_string();
    assert!(
        first_short.chars().all(|c| c.is_ascii_hexdigit()) && first_short.len() >= 4,
        "expected a hex short-id at the start of cleaned log output, got {first_short:?}"
    );

    memora()
        .args(["add", "--type", "project", "--content", "v2", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "c2"])
        .current_dir(path)
        .assert()
        .success();

    memora()
        .args(["rollback", "--to", &first_short])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Rolled back"));
}

#[test]
fn branch_create_and_list() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "preference", "--content", "p", "--source", "manual"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "first"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["branch", "experiment/x"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .arg("branch")
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("experiment/x"))
        .stdout(predicate::str::contains("main"));
    memora()
        .args(["switch", "experiment/x"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Switched"));
}


#[test]
fn promote_by_kind_then_diff_shows_status_change() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();

    memora().arg("init").current_dir(path).assert().success();

    memora()
        .args([
            "add",
            "--type",
            "assumption",
            "--content",
            "redis is the cache",
            "--source",
            "model-inference",
        ])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "initial guess"])
        .current_dir(path)
        .assert()
        .success();

    memora()
        .args(["promote", "--type", "assumption"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("ephemeral → stable"));
    memora()
        .args(["commit", "-m", "promote redis"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 promoted"));

    memora()
        .args(["diff", "HEAD~1", "HEAD", "--semantic"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("ephemeral → stable"))
        .stdout(predicate::str::contains("Semantic summary"));
}

#[test]
fn promote_requires_one_target() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["promote"])
        .current_dir(path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("specify exactly one"));
}

#[test]
fn diff_against_working_set_picks_up_uncommitted_changes() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "v1", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "first"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["add", "--type", "project", "--content", "v2", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["diff", "HEAD", "--working"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Added:"))
        .stdout(predicate::str::contains("v2"));
}


#[test]
fn merge_clean_three_way_via_cli() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();

    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "shared", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "base"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["branch", "feature"])
        .current_dir(path)
        .assert()
        .success();
    // diverge: feature side
    memora()
        .args(["switch", "feature"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args([
            "add", "--type", "semantic", "--content", "auth uses jwt", "--source", "code-read",
        ])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "feat"])
        .current_dir(path)
        .assert()
        .success();
    // diverge: main side
    memora()
        .args(["switch", "main"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args([
            "add",
            "--type",
            "preference",
            "--content",
            "verbose errors",
            "--source",
            "manual",
        ])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "pref"])
        .current_dir(path)
        .assert()
        .success();
    // merge feature into main
    memora()
        .args(["merge", "feature"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Merged"));
}

#[test]
fn merge_dry_run_does_not_change_anything() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "x", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "c1"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["branch", "feature"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["merge", "feature", "--dry-run"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("merge plan"));
}

#[test]
fn merge_already_up_to_date_via_cli() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "x", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "c1"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["branch", "feature"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["merge", "feature"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Already up to date"));
}


#[test]
fn session_lifecycle_via_cli() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();

    memora()
        .args(["session", "start", "--source", "claude_code"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Started session"));
    memora()
        .args(["add", "--type", "project", "--content", "uses Rust", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "first"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["session", "end"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Ended session"));

    // list shows the session, replay walks events.
    memora()
        .args(["session", "list"])
        .current_dir(path)
        .assert()
        .success();
}

#[test]
fn replay_streams_events() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["session", "start", "--source", "manual"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["add", "--type", "project", "--content", "x", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "c"])
        .current_dir(path)
        .assert()
        .success();
    // Replay before ending the session — should still show the events so far.
    memora()
        .args(["replay"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("session_started"))
        .stdout(predicate::str::contains("node_added"))
        .stdout(predicate::str::contains("commit_created"));
}

#[test]
fn export_claude_code_writes_file() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args([
            "add",
            "--type",
            "semantic",
            "--content",
            "auth uses jwt rs256",
            "--source",
            "code-read",
        ])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["commit", "-m", "first"])
        .current_dir(path)
        .assert()
        .success();
    memora()
        .args(["export", "--to", "claude-code"])
        .current_dir(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));
    let body = std::fs::read_to_string(path.join("CLAUDE.md")).unwrap();
    assert!(body.contains("auth uses jwt rs256"));
    assert!(body.contains("## Semantic"));
}

#[test]
fn export_json_to_stdout() {
    let tmp = tempdir().unwrap();
    let path = tmp.path();
    memora().arg("init").current_dir(path).assert().success();
    memora()
        .args(["add", "--type", "project", "--content", "rust", "--source", "code-read"])
        .current_dir(path)
        .assert()
        .success();
    let out = memora()
        .args(["export", "--to", "json", "--stdout"])
        .current_dir(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).unwrap();
    let cleaned = strip_ansi(&body);
    let parsed: serde_json::Value = serde_json::from_str(cleaned.trim()).unwrap();
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 1);
}
