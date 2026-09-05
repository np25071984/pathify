//! End-to-end tests for `pathify merge`.
//!
//! `ride.gpx` and `ride-watch.gpx` are the same ride recorded by two devices:
//! the watch's timestamps are two seconds later, its coordinates a few meters
//! off, and it kept recording for two points after the phone stopped.

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

/// Reads the merged output back through `info --json`.
fn info_of(merged: Vec<u8>) -> serde_json::Value {
    let output = pathify()
        .args(["info", "--json"])
        .write_stdin(merged)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).expect("merged output should be readable")
}

fn merge_fixtures(extra: &[&str]) -> Vec<u8> {
    let output = pathify()
        .arg("merge")
        .arg(fixture("ride.gpx"))
        .arg(fixture("ride-watch.gpx"))
        .args(extra)
        .output()
        .unwrap();
    assert!(output.status.success(), "merge failed: {output:?}");
    output.stdout
}

/// The headline case: two devices, one ride. The merged trace must not report
/// roughly double the distance, which is what an unmatched merge produces.
#[test]
fn reconciles_two_recordings_of_the_same_ride() {
    let original = info_of(std::fs::read(fixture("ride.gpx")).unwrap());
    let merged = info_of(merge_fixtures(&[]));

    // Ten shared moments collapse; the watch's two extra points are kept.
    assert_eq!(merged["points"], 12);
    assert_eq!(original["points"], 10);

    let (before, after) = (
        original["distance_m"].as_f64().unwrap(),
        merged["distance_m"].as_f64().unwrap(),
    );
    assert!(
        after < before * 1.5,
        "merged distance {after:.0} m suggests duplicates were not matched (original {before:.0} m)"
    );
    // The watch recorded a minute longer, so the span grows but does not double.
    assert!(merged["duration_s"].as_i64().unwrap() > original["duration_s"].as_i64().unwrap());
}

#[test]
fn reports_what_it_did_on_stderr_with_verbose() {
    pathify()
        .arg("merge")
        .arg(fixture("ride.gpx"))
        .arg(fixture("ride-watch.gpx"))
        .arg("--verbose")
        .assert()
        .success()
        .stderr(predicate::str::contains("reconciled 2 inputs"))
        .stderr(predicate::str::contains("10 matched as duplicates"));
}

/// `--no-dedup` must keep every point, which is also the shape of the bug this
/// command exists to avoid.
#[test]
fn no_dedup_keeps_every_point() {
    let merged = info_of(merge_fixtures(&["--no-dedup"]));
    assert_eq!(merged["points"], 22, "10 + 12 with nothing matched");
}

#[test]
fn a_tight_radius_stops_matching() {
    // The devices are several meters apart, so a one-meter radius matches none.
    let merged = info_of(merge_fixtures(&["--dedup-radius", "1"]));
    assert_eq!(merged["points"], 22);
}

/// Merging a file with itself is the sharpest test of the matching rule: too
/// loose and points vanish, too tight and they double.
#[test]
fn merging_a_file_with_itself_changes_nothing() {
    let output = pathify()
        .arg("merge")
        .arg(fixture("ride.gpx"))
        .arg(fixture("ride.gpx"))
        .output()
        .unwrap();
    assert!(output.status.success());

    let original = info_of(std::fs::read(fixture("ride.gpx")).unwrap());
    let merged = info_of(output.stdout);

    assert_eq!(merged["points"], original["points"]);
    assert_eq!(merged["segments"], original["segments"]);
    assert_eq!(merged["duration_s"], original["duration_s"]);
    let (a, b) = (
        original["distance_m"].as_f64().unwrap(),
        merged["distance_m"].as_f64().unwrap(),
    );
    assert!((a - b).abs() < 0.001, "{a} vs {b}");
}

#[test]
fn the_primary_input_supplies_the_coordinates() {
    let phone_first = info_of(merge_fixtures(&[]));
    let watch_first = info_of(merge_fixtures(&[
        "--primary",
        fixture("ride-watch.gpx").to_str().unwrap(),
    ]));

    // Same points either way, but taken from different devices, so the bounding
    // box differs in its low digits.
    assert_eq!(phone_first["points"], watch_first["points"]);
    assert_ne!(
        phone_first["bounds"]["min_lat"],
        watch_first["bounds"]["min_lat"]
    );
}

#[test]
fn a_primary_that_is_not_an_input_is_rejected() {
    pathify()
        .arg("merge")
        .arg(fixture("ride.gpx"))
        .arg(fixture("ride-watch.gpx"))
        .args(["--primary", "some-other-file.gpx"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not one of the inputs"));
}

#[test]
fn output_defaults_to_the_first_inputs_format() {
    let merged = merge_fixtures(&[]);
    let text = String::from_utf8(merged).unwrap();
    assert!(text.contains("<gpx"), "GPX in, GPX out");
}

#[test]
fn the_output_format_can_be_chosen() {
    let merged = merge_fixtures(&["--to", "csv"]);
    let text = String::from_utf8(merged).unwrap();
    assert!(text.starts_with("track,segment,lat,lon,ele,time"), "{text}");
}

#[test]
fn merge_needs_at_least_one_input() {
    pathify()
        .arg("merge")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

/// A merge is only worth doing across sources, so the whole point is that this
/// composes with the rest of the pipeline.
#[test]
fn merged_output_pipes_into_other_commands() {
    let merged = merge_fixtures(&["--to", "geojson"]);
    pathify()
        .args(["info", "-"])
        .write_stdin(merged)
        .assert()
        .success()
        .stdout(predicate::str::contains("format      geojson"));
}
