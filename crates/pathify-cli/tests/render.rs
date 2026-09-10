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

fn basemap_bytes() -> Vec<u8> {
    std::fs::read(fixture("basemap.png")).expect("the basemap fixture should exist")
}

/// The box `tests/fixtures/basemap.png` was generated for — `ride.gpx`'s own
/// bounds, which is what makes omitting `--bbox` correct in most tests here.
const RIDE_BBOX: &str = "-122.305600,47.653500,-122.274500,47.665100";

const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn assert_is_png(bytes: &[u8]) {
    assert!(
        bytes.starts_with(PNG_SIGNATURE),
        "expected a PNG on stdout, got {} bytes starting {:?}",
        bytes.len(),
        &bytes[..bytes.len().min(16)]
    );
}

#[test]
fn renders_a_trace_onto_a_basemap() {
    let output = pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

#[test]
fn fog_mode_also_produces_a_png() {
    let output = pathify()
        .args(["render", "--fog", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

/// The two modes must actually differ. Both produce a valid PNG, so a mode
/// flag that quietly did nothing would pass every other test in this file.
#[test]
fn the_two_modes_produce_different_images() {
    let render = |extra: &[&str]| {
        let mut command = pathify();
        command.arg("render").args(extra).arg("--basemap");
        command
            .arg(fixture("basemap.png"))
            .arg(fixture("ride.gpx"))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    assert_ne!(render(&[]), render(&["--fog"]));
}

/// The documented workflow: the basemap arrives on the pipe, because that is
/// where a `curl` leaves it.
#[test]
fn the_basemap_can_be_piped_in() {
    let output = pathify()
        .arg("render")
        .arg(fixture("ride.gpx"))
        .write_stdin(basemap_bytes())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

/// And the other way round, which is what keeps `render` composable with the
/// commands that emit a trace.
#[test]
fn the_trace_can_be_piped_in_instead() {
    let output = pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .write_stdin(std::fs::read(fixture("ride.gpx")).unwrap())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

/// Which input is which is decided by content, so the same pipeline works with
/// the trace coming out of another Pathify command.
#[test]
fn a_converted_trace_can_be_piped_in() {
    let geojson = pathify()
        .args(["convert", "--to", "geojson"])
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .write_stdin(geojson)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

/// A PNG on the pipe and a `--basemap` flag name two basemaps and no trace.
/// Picking one would be a guess.
#[test]
fn two_basemaps_and_no_trace_is_a_mistake_worth_reporting() {
    pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .write_stdin(basemap_bytes())
        .assert()
        .failure()
        .stderr(predicate::str::contains("given twice"));
}

/// Naming both inputs as files means stdin is never read, which is what every
/// other Pathify command does with a redundant pipe.
#[test]
fn naming_both_inputs_ignores_the_pipe() {
    pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .write_stdin(b"this is not read".to_vec())
        .assert()
        .success();
}

/// The basemap is the one thing Pathify will not fetch for itself, so the
/// error has to say where to get one.
#[test]
fn a_missing_basemap_says_where_to_get_one() {
    pathify()
        .arg("render")
        .arg(fixture("ride.gpx"))
        .write_stdin(Vec::new())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("no basemap")
                .and(predicate::str::contains("bbox"))
                .and(predicate::str::contains("--basemap")),
        );
}

/// Half the world's longitudes are negative, so this is the shape of nearly
/// every real invocation: without `allow_hyphen_values`, clap reads the value
/// as an unknown flag and the command is unusable west of Greenwich.
#[test]
fn a_bbox_with_negative_longitudes_parses() {
    let output = pathify()
        .args(["render", "--bbox", RIDE_BBOX, "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

#[test]
fn a_bbox_with_no_extent_is_refused_before_anything_is_drawn() {
    pathify()
        .args([
            "render",
            "--bbox",
            "-122.3056,47.6535,-122.3056,47.6535",
            "--basemap",
        ])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("no extent"));
}

#[test]
fn a_transposed_bbox_is_caught_by_the_range_checks() {
    // Latitude and longitude swapped: 47.65 is a fine longitude, but -122.3 is
    // not a latitude, and saying so beats rendering a map of the wrong ocean.
    pathify()
        .args([
            "render",
            "--bbox",
            "47.6535,-122.3056,47.6651,-122.2745",
            "--basemap",
        ])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("latitude"));
}

#[test]
fn a_trace_that_misses_the_basemap_is_reported() {
    // The basemap covers Seattle; this box says it covers London.
    pathify()
        .args(["render", "--bbox", "-0.13,51.50,-0.11,51.52", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("no part of the trace"));
}

#[test]
fn the_track_style_flags_are_accepted() {
    let output = pathify()
        .args([
            "render",
            "--track-color",
            "#e6194b",
            "--track-width",
            "6",
            "--basemap",
        ])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_is_png(&output);
}

/// Fog mode draws no line, so a stroke colour would silently do nothing. A
/// flag that is ignored is worse than one that is refused.
#[test]
fn styling_the_track_conflicts_with_fog() {
    pathify()
        .args(["render", "--fog", "--track-color", "#e6194b", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

/// `--reveal` measures ground distance, so it takes its unit inline like every
/// other measured flag: a corridor given in feet has to clear the same ground
/// as the meters it converts to.
#[test]
fn a_reveal_in_feet_matches_the_same_reveal_in_meters() {
    let fogged = |reveal: &str| {
        pathify()
            .args(["render", "--fog", "--reveal", reveal, "--bbox", RIDE_BBOX])
            .args(["--basemap"])
            .arg(fixture("basemap.png"))
            .arg(fixture("ride.gpx"))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    let feet = fogged("100ft");
    assert_is_png(&feet);
    assert_eq!(feet, fogged("30.48"));
    // And the radius mattered, or the comparison proves nothing.
    assert_ne!(feet, fogged("10"));
}

/// A bare number is meters wherever the host is. The locale picks the default
/// when the flag is absent; it must not re-interpret a number the user wrote,
/// or the same command would clear a corridor three times over on one machine
/// and not the other.
#[test]
fn a_bare_reveal_means_meters_on_an_imperial_host() {
    let fogged = |locale: &str| {
        pathify()
            .env("LC_ALL", locale)
            .args(["render", "--fog", "--reveal", "30", "--bbox", RIDE_BBOX])
            .args(["--basemap"])
            .arg(fixture("basemap.png"))
            .arg(fixture("ride.gpx"))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    assert_eq!(fogged("en_US.UTF-8"), fogged("en_GB.UTF-8"));
}

/// A duration unit on a distance is a typo, not a value to guess at.
#[test]
fn a_duration_unit_on_the_reveal_is_refused() {
    pathify()
        .args(["render", "--fog", "--reveal", "5min", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown distance unit"));
}

#[test]
fn the_fog_settings_only_mean_something_with_fog() {
    for flag in [["--reveal", "300"], ["--fog-opacity", "0.5"]] {
        pathify()
            .args(["render"])
            .args(flag)
            .args(["--basemap"])
            .arg(fixture("basemap.png"))
            .arg(fixture("ride.gpx"))
            .assert()
            .failure()
            .stderr(predicate::str::contains("--fog"));
    }
}

#[test]
fn an_out_of_range_fog_opacity_is_refused() {
    pathify()
        .args(["render", "--fog", "--fog-opacity", "1.5", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("between 0 and 1"));
}

#[test]
fn a_bad_colour_names_the_format_it_wanted() {
    pathify()
        .args(["render", "--track-color", "blue", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("#RRGGBB"));
}

/// A basemap that will not decode is bad input, not an environment failure, so
/// it exits 1 — even when the underlying cause is a truncated read.
#[test]
fn a_basemap_that_is_not_a_png_exits_as_bad_input() {
    pathify()
        .args(["render", "--basemap"])
        .arg(fixture("ride.gpx"))
        .arg(fixture("ride.gpx"))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("PNG"));
}

#[test]
fn the_output_can_go_to_a_file() {
    let directory = std::env::temp_dir().join(format!("pathify-render-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let destination = directory.join("out.png");

    pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .arg("-o")
        .arg(&destination)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    assert_is_png(&std::fs::read(&destination).unwrap());
    std::fs::remove_dir_all(&directory).ok();
}

/// The image is the shape its box implies, so the usual run is quiet. A
/// warning that fired every time would be ignored when it mattered.
#[test]
fn a_matching_basemap_renders_without_a_warning() {
    pathify()
        .args(["render", "--basemap"])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

/// The likeliest real mistake: an image downloaded for one area, rendered with
/// another area's box. The proportions give it away.
#[test]
fn a_basemap_of_the_wrong_shape_is_flagged() {
    pathify()
        .args([
            "render",
            "--bbox",
            "-122.3056,47.6535,-122.2745,47.7",
            "--basemap",
        ])
        .arg(fixture("basemap.png"))
        .arg(fixture("ride.gpx"))
        .assert()
        .success()
        .stderr(predicate::str::contains("stretched"));
}

#[test]
fn the_help_documents_both_modes_and_the_no_network_stance() {
    pathify()
        .args(["render", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--fog")
                .and(predicate::str::contains("--basemap"))
                .and(predicate::str::contains("--bbox"))
                .and(predicate::str::contains("does not"))
                .or(predicate::str::contains("never downloads")),
        );
}
