//! End-to-end tests for `pathify clean`.
//!
//! The redaction tests here check the published bytes, not the parsed model.
//! That is deliberate: what leaks a home address is what ends up in the file
//! someone shares, and a check on the in-memory trace would not catch a
//! coordinate that survived in, say, a metadata field.

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

fn clean_fixture(name: &str, extra: &[&str]) -> Vec<u8> {
    let output = pathify()
        .arg("clean")
        .arg(fixture(name))
        .args(extra)
        .output()
        .unwrap();
    assert!(output.status.success(), "clean failed: {output:?}");
    output.stdout
}

fn info_of(bytes: Vec<u8>) -> serde_json::Value {
    let output = pathify()
        .args(["info", "--json"])
        .write_stdin(bytes)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).expect("cleaned output should be readable")
}

/// The fixture is a good recording, so a default clean must not touch it.
/// Silently deleting points from clean data would be its own bug.
#[test]
fn a_clean_trace_passes_through_unchanged() {
    let original = info_of(std::fs::read(fixture("ride.gpx")).unwrap());
    let cleaned = info_of(clean_fixture("ride.gpx", &[]));

    assert_eq!(cleaned["points"], original["points"]);
    assert_eq!(cleaned["segments"], original["segments"]);
    assert_eq!(cleaned["distance_m"], original["distance_m"]);
}

#[test]
fn removes_a_fix_that_teleports() {
    let original = info_of(std::fs::read(fixture("ride-drift.gpx")).unwrap());
    let cleaned = info_of(clean_fixture("ride-drift.gpx", &[]));

    assert_eq!(original["points"], 11);
    assert_eq!(cleaned["points"], 10, "the bad fix should be gone");

    // The outlier sits far north of the route and inflates distance hugely.
    let (before, after) = (
        original["distance_m"].as_f64().unwrap(),
        cleaned["distance_m"].as_f64().unwrap(),
    );
    assert!(before > 40_000.0, "fixture should contain a big jump");
    assert!(after < 3_000.0, "distance still inflated: {after:.0} m");
}

#[test]
fn drift_filtering_can_be_disabled() {
    let cleaned = info_of(clean_fixture("ride-drift.gpx", &["--no-drift-filter"]));
    assert_eq!(cleaned["points"], 11);
}

/// Nothing should be redacted unless it was asked for.
#[test]
fn a_default_clean_redacts_nothing() {
    pathify()
        .arg("clean")
        .arg(fixture("ride.gpx"))
        .arg("--verbose")
        .assert()
        .success()
        .stderr(predicate::str::contains("kept all 10 points"));
}

/// The guarantee that matters: after `--redact-around`, the fenced coordinates
/// must not appear in the published file at all.
#[test]
fn redacted_coordinates_do_not_appear_in_the_output() {
    // Fence the start of the fixture, 47.6535,-122.3056, with a 300 m radius.
    let output = clean_fixture(
        "ride.gpx",
        &["--redact-around", "47.6535,-122.3056,300", "--to", "csv"],
    );
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains("47.6535"),
        "the fenced point survived:\n{text}"
    );
    assert!(
        !text.contains("47.6542"),
        "a point inside the fence survived:\n{text}"
    );
    // Points well outside the fence are untouched.
    assert!(
        text.contains("47.6651"),
        "unfenced points were lost:\n{text}"
    );
}

/// Redaction from the middle of a run must leave a gap, not draw a straight
/// line through the hidden area.
#[test]
fn redaction_splits_the_segment_it_cuts() {
    let original = info_of(std::fs::read(fixture("ride.gpx")).unwrap());
    let cleaned = info_of(clean_fixture(
        "ride.gpx",
        &["--redact-around", "47.6561,-122.2972,200"],
    ));

    assert_eq!(original["segments"], 2);
    assert_eq!(
        cleaned["segments"], 3,
        "the cut should split its segment rather than close the gap"
    );
}

#[test]
fn trim_ends_hides_both_endpoints() {
    let output = clean_fixture("ride.gpx", &["--trim-ends", "300", "--to", "csv"]);
    let text = String::from_utf8(output).unwrap();

    assert!(!text.contains("47.6535"), "the start survived:\n{text}");
    assert!(!text.contains("47.6651"), "the end survived:\n{text}");
}

#[test]
fn several_fences_can_be_given() {
    let output = clean_fixture(
        "ride.gpx",
        &[
            "--redact-around",
            "47.6535,-122.3056,200",
            "--redact-around",
            "47.6651,-122.2745,200",
            "--to",
            "csv",
        ],
    );
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("47.6535"), "{text}");
    assert!(!text.contains("47.6651"), "{text}");
}

#[test]
fn reports_each_kind_of_removal_with_verbose() {
    pathify()
        .arg("clean")
        .arg(fixture("ride-drift.gpx"))
        .args(["--trim-ends", "300", "--verbose"])
        .assert()
        .success()
        .stderr(predicate::str::contains("bad fixes"))
        .stderr(predicate::str::contains("near the ends"));
}

#[test]
fn a_malformed_fence_explains_the_expected_shape() {
    pathify()
        .arg("clean")
        .arg(fixture("ride.gpx"))
        .args(["--redact-around", "47.6535,-122.3056"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("LAT,LON,RADIUS"));
}

/// A negative radius would fence nothing and quietly publish the location the
/// user asked to hide, so it has to be refused rather than accepted.
#[test]
fn a_negative_radius_is_refused() {
    pathify()
        .arg("clean")
        .arg(fixture("ride.gpx"))
        .args(["--redact-around", "47.6535,-122.3056,-100"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("zero or more"));
}

#[test]
fn an_out_of_range_coordinate_is_refused() {
    pathify()
        .arg("clean")
        .arg(fixture("ride.gpx"))
        .args(["--redact-around", "947.6,-122.3,100"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("outside -90..=90"));
}

#[test]
fn cleaning_composes_in_a_pipeline() {
    let bytes = std::fs::read(fixture("ride-drift.gpx")).unwrap();
    pathify()
        .args(["clean", "-", "--to", "geojson"])
        .write_stdin(bytes)
        .assert()
        .success()
        .stdout(predicate::str::contains("FeatureCollection"));
}

/// As with `merge`, the long help only reaches the user from a doc comment on
/// the variant. This one matters more: it is where the opt-in nature of
/// redaction is explained.
#[test]
fn the_long_help_explains_the_two_kinds_of_removal() {
    pathify()
        .args(["clean", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Drift filtering runs by default"))
        .stdout(predicate::str::contains(
            "Redaction never runs unless you ask",
        ))
        .stdout(predicate::str::contains("--trim-ends"));
}
