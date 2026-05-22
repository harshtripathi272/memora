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
