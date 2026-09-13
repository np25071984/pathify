//! End-to-end tests for `pathify takeout`.
//!
//! `tests/fixtures/takeout-sample/` is a synthetic Google Health export, laid
//! out exactly like a real one and four kilobytes instead of two gigabytes.
//! Its README lists the quirks it reproduces and why each one is there.
//!
//! Every test runs against both the directory and a Zip built from it in a
//! temporary file, because `takeout` accepts either and the two take different
//! paths through `Archive`.

use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/takeout-sample")
}

fn pathify() -> Command {
    Command::cargo_bin("pathify").expect("the pathify binary should build")
}

/// The fixture zipped up, so the Zip reader is exercised on the same data.
///
/// Built here rather than committed, so the fixture stays a set of files
/// anyone can read in a diff instead of a binary blob nobody can review.
struct Zipped(PathBuf);

impl Zipped {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("pathify-takeout-{}-{name}.zip", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        add_directory(&mut writer, &fixture_dir(), Path::new(""));
        writer.finish().unwrap();
        Self(path)
    }
}

impl Drop for Zipped {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn add_directory(writer: &mut zip::ZipWriter<std::fs::File>, dir: &Path, prefix: &Path) {
    let mut items: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    items.sort_by_key(std::fs::DirEntry::path);
    for item in items {
        let inside = prefix.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            add_directory(writer, &item.path(), &inside);
        } else {
            writer
                .start_file(
                    inside.to_string_lossy(),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(&std::fs::read(item.path()).unwrap())
                .unwrap();
        }
    }
}

/// Run the same arguments against the unpacked directory and the Zip, and
/// hand back both outputs so a test can assert on them together.
fn both_forms(args: &[&str], name: &str) -> [std::process::Output; 2] {
    // The Zip is held in scope until both commands have run, and deleted
    // when it goes out of it.
    let zipped = Zipped::new(name);
    [fixture_dir(), zipped.0.clone()].map(|archive| {
        pathify()
            .arg("takeout")
            .arg(archive)
            .args(args)
            .output()
            .unwrap()
    })
}

/// A path in the temp directory for one test to write into.
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("pathify-takeout-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_file(&path);
    path
}

/// The file names in a directory, sorted, so a test can assert on the batch.
fn listing(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the output directory should have been created")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn info_of(trace: &[u8]) -> serde_json::Value {
    let output = pathify()
        .args(["info", "--json"])
        .write_stdin(trace.to_vec())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).expect("takeout output should be readable")
}

/// The table carries a header, because two bare numbers per row leave the
/// reader to guess what the second one counts.
#[test]
fn list_reports_every_type_under_named_columns() {
    for output in both_forms(&["--list"], "list") {
        assert!(output.status.success(), "{output:?}");
        let listing = String::from_utf8(output.stdout).unwrap();
        // Compared with the column padding collapsed, so widening a column
        // does not break the test.
        let rows: Vec<String> = listing
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let has = |row: &str| rows.iter().any(|line| line == row);

        assert!(has("activity type logs with GPS"), "{listing}");
        assert!(has("Walk 2 1"), "{listing}");
        assert!(has("Outdoor Bike 1 1"), "{listing}");
        assert!(has("Workout 1 0"), "{listing}");
        assert!(has("5 logs, 3 with GPS, 2 days of recording"), "{listing}");
    }
}

/// The same listing, for a program: same rows, same totals, as JSON.
#[test]
fn list_json_carries_the_same_counts_as_the_table() {
    for output in both_forms(&["--list", "--json"], "listjson") {
        assert!(output.status.success(), "{output:?}");
        let listing: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("--list --json should emit JSON");

        assert_eq!(listing["types"][0]["name"], "Walk");
        assert_eq!(listing["types"][0]["logs"], 2);
        assert_eq!(listing["types"][0]["with_gps"], 1);
        assert_eq!(listing["total_logs"], 5);
        assert_eq!(listing["total_with_gps"], 3);
        assert_eq!(listing["days_of_recording"], 2);

        let names: Vec<&str> = listing["types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|activity| activity["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Walk", "Outdoor Bike", "Swim", "Workout"]);
    }
}

/// One file per activity, named after the activity's own start and type, in
/// a directory made for them. A caller importing each outing as its own
/// record never has to teach its parser about multi-activity files.
#[test]
fn per_activity_writes_one_file_per_activity() {
    let directory = scratch("per-activity");
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk,outdoor bike", "--per-activity", "-o"])
        .arg(&directory)
        .assert()
        .success()
        // The result is the directory; stdout stays clean, as it does for
        // any other `-o`.
        .stdout(predicate::str::is_empty());

    assert_eq!(
        listing(&directory),
        [
            "20260711T113000Z-walk.gpx",
            "20260711T235500Z-outdoor-bike.gpx"
        ]
    );

    // Each one is a whole trace of its own, holding that activity and
    // nothing else — the same points the combined output carries.
    let walk = info_of(&std::fs::read(directory.join("20260711T113000Z-walk.gpx")).unwrap());
    assert_eq!(walk["tracks"], 1);
    assert_eq!(walk["points"], 5);
    assert_eq!(walk["segments"], 2);
    assert_eq!(walk["name"], "Walk 2026-07-11");

    let bike =
        info_of(&std::fs::read(directory.join("20260711T235500Z-outdoor-bike.gpx")).unwrap());
    assert_eq!(bike["tracks"], 1);
    assert_eq!(bike["points"], 6);

    std::fs::remove_dir_all(directory).ok();
}

/// Running it twice rewrites the same files rather than piling up a second
/// copy under different names: the names come from the activities.
#[test]
fn a_second_run_rewrites_its_own_output() {
    let directory = scratch("per-activity-twice");
    for _ in 0..2 {
        pathify()
            .arg("takeout")
            .arg(fixture_dir())
            .args(["--type", "walk", "--per-activity", "-o"])
            .arg(&directory)
            .assert()
            .success();
    }
    assert_eq!(listing(&directory), ["20260711T113000Z-walk.gpx"]);
    std::fs::remove_dir_all(directory).ok();
}

/// `--to` reaches the batch as well, extension and all.
#[test]
fn per_activity_files_are_written_in_the_requested_format() {
    let directory = scratch("per-activity-csv");
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--per-activity", "--to", "csv", "-o"])
        .arg(&directory)
        .assert()
        .success();

    assert_eq!(listing(&directory), ["20260711T113000Z-walk.csv"]);
    let written = std::fs::read_to_string(directory.join("20260711T113000Z-walk.csv")).unwrap();
    assert!(
        written.starts_with("track,segment,lat,lon,ele,time"),
        "{written}"
    );
    std::fs::remove_dir_all(directory).ok();
}

/// A batch needs somewhere to put the files, and a file is not somewhere.
#[test]
fn per_activity_wants_a_directory_and_says_so() {
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--per-activity"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--output"));

    let file = scratch("per-activity-file");
    std::fs::write(&file, "not a directory").unwrap();
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--per-activity", "-o"])
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("directory"));
    std::fs::remove_file(file).ok();
}

/// The headline: an activity comes out sliced from its day file, with the
/// other device and the other outings left behind.
#[test]
fn extracting_a_type_yields_one_track_per_activity() {
    for output in both_forms(&["--type", "walk"], "walk") {
        assert!(output.status.success(), "{output:?}");
        let info = info_of(&output.stdout);
        assert_eq!(info["tracks"], 1);
        assert_eq!(info["points"], 5);
        // The six-minute hole in the middle is a pause, not a walk in a
        // straight line.
        assert_eq!(info["segments"], 2);
    }
}

/// Case, spaces and underscores are all the same request.
#[test]
fn a_type_filter_matches_however_it_is_spelled() {
    for spelling in [
        "outdoor bike",
        "Outdoor Bike",
        "outdoor_bike",
        "OUTDOOR-BIKE",
    ] {
        let output = pathify()
            .arg("takeout")
            .arg(fixture_dir())
            .args(["--type", spelling])
            .output()
            .unwrap();
        assert!(output.status.success(), "{spelling}: {output:?}");
        assert_eq!(info_of(&output.stdout)["points"], 6, "{spelling}");
    }
}

/// GPX by default, because a Zip has no format of its own to inherit.
#[test]
fn the_default_output_is_gpx_and_the_flags_still_win() {
    let output = pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("<gpx"));
    assert_eq!(info_of(&output.stdout)["source_format"], "gpx");

    let output = pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--to", "geojson"])
        .output()
        .unwrap();
    assert_eq!(info_of(&output.stdout)["source_format"], "geojson");
}

/// `-o walks.csv` should not also need `--to csv`.
#[test]
fn an_output_extension_chooses_the_format() {
    let out = std::env::temp_dir().join(format!("pathify-takeout-{}.csv", std::process::id()));
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "-o"])
        .arg(&out)
        .assert()
        .success();
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(
        written.starts_with("track,segment,lat,lon,ele,time"),
        "{written}"
    );
    std::fs::remove_file(out).ok();
}

/// A window that runs past midnight UTC is stitched from two day files.
#[test]
fn a_ride_across_midnight_keeps_both_halves() {
    for output in both_forms(&["--type", "outdoor bike"], "midnight") {
        let info = info_of(&output.stdout);
        assert_eq!(info["tracks"], 1);
        assert_eq!(info["segments"], 1);
        assert_eq!(info["points"], 6);
    }
}

/// A misspelled type is an error that lists the real ones. Emitting an empty
/// trace instead looks exactly like an archive that held nothing.
#[test]
fn an_unknown_type_lists_the_ones_that_exist() {
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "hiking"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("hiking").and(predicate::str::contains("Outdoor Bike")));
}

/// A log can claim GPS with no day file behind it, and `--verbose` has to say
/// so rather than let a short trace pass for a complete one.
#[test]
fn a_missing_day_file_is_reported_not_hidden() {
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "swim", "--verbose"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("no day file for 2026-07-20")
                .and(predicate::str::contains("produced no points")),
        );
}

/// Two devices recorded the same walk. Keeping both would report twice the
/// distance, so one wins — and `--source` says which.
#[test]
fn only_one_recording_device_comes_out() {
    let busiest = pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--verbose"])
        .output()
        .unwrap();
    assert_eq!(info_of(&busiest.stdout)["points"], 5);
    assert!(
        String::from_utf8_lossy(&busiest.stderr).contains("kept `Pathify Phone`"),
        "{busiest:?}"
    );

    let named = pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--source", "pathify watch"])
        .output()
        .unwrap();
    assert_eq!(info_of(&named.stdout)["points"], 3);
}

#[test]
fn naming_a_device_that_never_recorded_says_which_ones_did() {
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--source", "Garmin"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Garmin").and(predicate::str::contains("Pathify Phone")));
}

/// The segment gap carries its unit inline, like every other measured flag.
#[test]
fn the_segment_gap_takes_a_unit() {
    let output = pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk", "--segment-gap", "10min"])
        .output()
        .unwrap();
    assert_eq!(info_of(&output.stdout)["segments"], 1);
}

/// An export with no location data in it has to name what it does hold: the
/// fix is a different export, not a different command.
#[test]
fn an_export_without_location_data_says_what_it_found() {
    let dir = std::env::temp_dir().join(format!("pathify-takeout-empty-{}", std::process::id()));
    let inside = dir.join("Takeout/YouTube and YouTube Music/history");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::write(inside.join("watch-history.json"), "[]").unwrap();

    pathify()
        .arg("takeout")
        .arg(&dir)
        .args(["--type", "walk"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("YouTube and YouTube Music"));

    std::fs::remove_dir_all(dir).ok();
}

/// Without a terminal to ask at, `takeout` says how to make the choice on the
/// command line instead of painting a menu into a pipe.
#[test]
fn no_type_and_no_terminal_asks_for_the_flag() {
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--type").and(predicate::str::contains("--list")));
}

/// Exit codes are contractual: an unreadable archive is an I/O failure, and a
/// readable one Pathify cannot make sense of is bad input.
#[test]
fn an_unreadable_archive_exits_two_and_a_corrupt_one_exits_one() {
    pathify()
        .args(["takeout", "/nonexistent/pathify/takeout.zip", "--list"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("takeout.zip"));

    let corrupt = std::env::temp_dir().join(format!(
        "pathify-takeout-corrupt-{}.zip",
        std::process::id()
    ));
    std::fs::write(
        &corrupt,
        b"PK\x03\x04 and then nothing that follows the spec",
    )
    .unwrap();
    pathify()
        .arg("takeout")
        .arg(&corrupt)
        .arg("--list")
        .assert()
        .code(1);
    std::fs::remove_file(corrupt).ok();
}

/// The privacy promise, as a test. A Takeout export carries sleep, glucose
/// and menstrual health beside the locations; `takeout` reads the exercise
/// logs and the GPS day files and nothing else.
#[test]
fn nothing_outside_the_location_data_reaches_the_output() {
    for output in both_forms(&["--type", "walk", "--verbose"], "privacy") {
        let everything = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !everything.contains("SENSITIVE-HEALTH-DATA"),
            "health data outside the location files reached the output"
        );
        // Nor does the `tcxLink` that the exercise logs carry: Pathify has no
        // network code and must never hand anyone a fitbit.com URL to follow.
        assert!(!everything.contains("fitbit.com"), "{everything}");
    }
}

/// The archive is only ever read.
#[test]
fn the_archive_is_left_exactly_as_it_was() {
    let before: Vec<_> = walk(&fixture_dir());
    pathify()
        .arg("takeout")
        .arg(fixture_dir())
        .args(["--type", "walk"])
        .assert()
        .success();
    assert_eq!(before, walk(&fixture_dir()));
}

fn walk(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut items: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    items.sort_by_key(std::fs::DirEntry::path);
    for item in items {
        if item.file_type().unwrap().is_dir() {
            files.extend(walk(&item.path()));
        } else {
            files.push((item.path(), std::fs::read(item.path()).unwrap()));
        }
    }
    files
}
