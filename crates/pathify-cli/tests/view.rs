//! End-to-end tests for `pathify view`.
//!
//! The map itself cannot be driven from here — there is no terminal to paint
//! into — so what is checked is the boundary around it: that `view` refuses a
//! redirected stdout clearly rather than failing deep inside a terminal
//! library, and that it says so before spending any time on the file. The
//! projection, viewport, keymap, and state machine are all unit-tested in
//! `pathify-tui`, where no terminal is needed.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn pathify() -> Command {
    Command::cargo_bin("pathify").expect("the pathify binary should build")
}

/// Tests always run with stdout piped, so this is the guard firing.
#[test]
fn refuses_a_redirected_stdout_with_an_explanation() {
    pathify()
        .arg("view")
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("needs a terminal"))
        .stderr(predicate::str::contains("pathify info"))
        .stdout(predicate::str::is_empty());
}

/// The guard runs before the file is opened, so a bad path is not what the
/// user hears about first — the terminal is the real problem.
#[test]
fn the_terminal_check_comes_before_reading_the_file() {
    pathify()
        .arg("view")
        .arg("/nonexistent/ride.gpx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("needs a terminal"))
        .stderr(predicate::str::contains("ride.gpx").not());
}

#[test]
fn view_is_listed_in_help() {
    pathify()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("interactive terminal map"));
}

/// The keys are only discoverable if `--help` mentions them, since the map
/// cannot be explored from a script.
#[test]
fn view_help_documents_the_keys_and_the_piping_rule() {
    pathify()
        .args(["view", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hjkl"))
        .stdout(predicate::str::contains("zoom"))
        .stdout(predicate::str::contains("q to quit"))
        .stdout(predicate::str::contains("cannot be piped"));
}

#[test]
fn view_accepts_a_format_override_flag() {
    pathify()
        .args(["view", "--from", "geojson", "-"])
        .write_stdin("{}")
        .assert()
        // Still the terminal check, but the flag parsed rather than erroring.
        .failure()
        .stderr(predicate::str::contains("needs a terminal"));
}

#[test]
fn an_unknown_format_is_still_rejected_by_the_parser() {
    pathify()
        .args(["view", "--from", "tcx"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown format"));
}
