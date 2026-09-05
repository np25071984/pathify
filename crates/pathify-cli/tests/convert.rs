//! End-to-end tests for `pathify convert`.

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

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pathify-convert-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn converts_gpx_to_geojson_on_stdout() {
    let output = pathify()
        .args(["convert", "--to", "geojson"])
        .arg(fixture("ride.gpx"))
        .output()
        .unwrap();
    assert!(output.status.success());

    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("convert --to geojson should emit valid JSON");
    assert_eq!(parsed["type"], "FeatureCollection");
    let geometry = &parsed["features"][0]["geometry"];
    // Two segments in the fixture, so a MultiLineString rather than a LineString.
    assert_eq!(geometry["type"], "MultiLineString");
    assert_eq!(geometry["coordinates"][0].as_array().unwrap().len(), 6);
    assert_eq!(geometry["coordinates"][1].as_array().unwrap().len(), 4);
    // Timestamps have somewhere to live even though GeoJSON has no native slot.
    assert!(parsed["features"][0]["properties"]["coordTimes"].is_array());
}

#[test]
fn converts_gpx_to_csv_with_the_documented_header() {
    let output = pathify()
        .args(["convert", "--to", "csv"])
        .arg(fixture("ride.gpx"))
        .output()
        .unwrap();
    assert!(output.status.success());

    let text = String::from_utf8(output.stdout).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), "track,segment,lat,lon,ele,time");
    assert_eq!(lines.clone().count(), 10);
    // The segment break in the fixture survives as a distinct segment index.
    assert!(text.contains("0,1,47.6612"), "{text}");
}

/// The point of one shared model: metrics computed from a converted file must
/// match the original, or `convert` is quietly losing data.
#[test]
fn conversion_preserves_the_metrics_info_reports() {
    let original = pathify()
        .args(["info", "--json"])
        .arg(fixture("ride.gpx"))
        .output()
        .unwrap();
    let original: serde_json::Value = serde_json::from_slice(&original.stdout).unwrap();

    for format in ["geojson", "csv"] {
        let converted = pathify()
            .args(["convert", "--to", format])
            .arg(fixture("ride.gpx"))
            .output()
            .unwrap();
        assert!(converted.status.success(), "convert --to {format} failed");

        let info = pathify()
            .args(["info", "--json", "--from", format])
            .write_stdin(converted.stdout)
            .output()
            .unwrap();
        let round_tripped: serde_json::Value = serde_json::from_slice(&info.stdout)
            .unwrap_or_else(|e| panic!("info on converted {format} output failed: {e}"));

        assert_eq!(
            round_tripped["points"], original["points"],
            "{format} points"
        );
        assert_eq!(
            round_tripped["segments"], original["segments"],
            "{format} segments: a lost segment break invents distance"
        );
        assert_eq!(
            round_tripped["duration_s"], original["duration_s"],
            "{format} duration: timestamps were lost"
        );
        assert_eq!(
            round_tripped["elevation"]["gain_m"], original["elevation"]["gain_m"],
            "{format} elevation gain"
        );

        let (a, b) = (
            original["distance_m"].as_f64().unwrap(),
            round_tripped["distance_m"].as_f64().unwrap(),
        );
        assert!((a - b).abs() < 0.5, "{format} distance: {a} vs {b}");
    }
}

#[test]
fn writes_to_a_file_with_output() {
    let dir = temp_dir("out");
    let destination = dir.join("ride.geojson");

    pathify()
        .args(["convert", "--to", "geojson", "-o"])
        .arg(&destination)
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let written = std::fs::read_to_string(&destination).unwrap();
    assert!(written.contains("FeatureCollection"), "{written}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `convert ride.gpx -o ride.csv` should not need `--to` as well.
#[test]
fn infers_the_target_format_from_the_output_extension() {
    let dir = temp_dir("infer");
    let destination = dir.join("ride.csv");

    pathify()
        .arg("convert")
        .arg(fixture("ride.gpx"))
        .arg("-o")
        .arg(&destination)
        .assert()
        .success();

    let written = std::fs::read_to_string(&destination).unwrap();
    assert!(
        written.starts_with("track,segment,lat,lon,ele,time"),
        "{written}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn converts_from_stdin_through_a_pipeline() {
    let bytes = std::fs::read(fixture("ride.gpx")).unwrap();
    pathify()
        .args(["convert", "--to", "csv"])
        .write_stdin(bytes)
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "track,segment,lat,lon,ele,time",
        ));
}

#[test]
fn without_a_target_format_it_says_how_to_supply_one() {
    pathify()
        .arg("convert")
        .arg(fixture("ride.gpx"))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--to"));
}

#[test]
fn converting_to_an_unimplemented_format_says_so() {
    pathify()
        .args(["convert", "--to", "fit"])
        .arg(fixture("ride.gpx"))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not implemented yet"));
}
