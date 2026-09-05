//! End-to-end tests for `pathify info`.
//!
//! These drive the real binary rather than calling the library, because the
//! contract being checked here is the CLI's: what lands on stdout vs. stderr,
//! and what the exit code is. That is what a shell pipeline depends on.

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

#[test]
fn reports_metrics_for_a_gpx_file() {
    pathify()
        .arg("info")
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Burke-Gilman morning"))
        .stdout(predicate::str::contains("points      10"))
        .stdout(predicate::str::contains("segments    2"))
        .stdout(predicate::str::contains("distance"))
        .stdout(predicate::str::contains("elevation"));
}

#[test]
fn reads_from_stdin_when_given_a_dash() {
    let bytes = std::fs::read(fixture("ride.gpx")).unwrap();
    pathify()
        .arg("info")
        .arg("-")
        .write_stdin(bytes)
        .assert()
        .success()
        .stdout(predicate::str::contains("points      10"));
}

/// Reading from stdin with no filename to go on exercises content sniffing —
/// the piece that makes Pathify usable mid-pipeline.
#[test]
fn detects_format_from_piped_content_without_a_filename() {
    let bytes = std::fs::read(fixture("ride.gpx")).unwrap();
    pathify()
        .arg("info")
        .write_stdin(bytes)
        .assert()
        .success()
        .stdout(predicate::str::contains("format      gpx"));
}

#[test]
fn json_output_is_machine_readable() {
    let output = pathify()
        .arg("info")
        .arg(fixture("ride.gpx"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output should parse as JSON");
    assert_eq!(parsed["points"], 10);
    assert_eq!(parsed["segments"], 2);
    assert_eq!(parsed["tracks"], 1);
    assert_eq!(parsed["source_format"], "gpx");
    assert_eq!(parsed["duration_s"], 390);
    assert!(parsed["distance_m"].as_f64().unwrap() > 0.0);
    assert!(parsed["elevation"]["gain_m"].as_f64().unwrap() > 0.0);
}

/// The elevation threshold has to actually reach the calculation: a huge floor
/// should flatten the reported climb to nothing.
#[test]
fn elevation_threshold_is_applied() {
    let output = pathify()
        .arg("info")
        .arg(fixture("ride.gpx"))
        .args(["--json", "--elevation-threshold", "1000"])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["elevation"]["gain_m"], 0.0);
    assert_eq!(parsed["elevation"]["loss_m"], 0.0);
}

#[test]
fn a_missing_file_fails_with_the_io_exit_code() {
    pathify()
        .arg("info")
        .arg("/nonexistent/ride.gpx")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ride.gpx"))
        .stdout(predicate::str::is_empty());
}

#[test]
fn unparseable_input_fails_with_the_data_exit_code() {
    pathify()
        .arg("info")
        .arg("--from")
        .arg("gpx")
        .write_stdin("<gpx><trk>")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("GPX"))
        .stdout(predicate::str::is_empty());
}

/// Content that isn't any known format should say so and point at `--from`,
/// rather than guessing and reporting nonsense.
#[test]
fn undetectable_input_explains_how_to_fix_it() {
    pathify()
        .arg("info")
        .write_stdin("just some text")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--from"));
}

#[test]
fn a_format_without_an_adapter_says_so_plainly() {
    pathify()
        .arg("info")
        .arg("--from")
        .arg("fit")
        .write_stdin("whatever")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("FIT"))
        .stderr(predicate::str::contains("not implemented yet"));
}
