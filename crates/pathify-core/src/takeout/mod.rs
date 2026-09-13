//! Reading GPS traces out of a Google Takeout export.
//!
//! Takeout is not one format. What an archive holds depends on which products
//! were ticked when it was requested, and the layout of each has changed more
//! than once, so [`Archive::backend`] decides what it is looking at rather
//! than assuming. Two backends exist in principle:
//!
//! - **Google Health / Fitbit** ([`fitbit`]) — implemented. Per-day GPS CSVs
//!   joined to exercise logs, which is what says that a stretch of one day was
//!   a bike ride.
//! - **Location History / Timeline** — detected and named in the error, not
//!   yet read. The seam is [`Backend::Timeline`] and a `timeline` module
//!   alongside `fitbit`.
//!
//! # Privacy
//!
//! A Takeout export is unusually sensitive: alongside the location data sits
//! sleep, heart rate, glucose and menstrual health. Only the two file families
//! [`fitbit`] names are ever opened, nothing else is read out of the archive,
//! and the archive itself is never modified. There is no network code here or
//! anywhere else in Pathify, so nothing can leave the machine — in particular
//! the `tcxLink` field of an exercise log, which is a `fitbit.com` URL, is
//! deliberately ignored.

pub mod archive;
pub mod fitbit;

pub use archive::{Archive, Backend, Entry};
pub use fitbit::{
    Activity, ActivityLog, ActivityType, Catalog, Options, Report, extract, extract_each,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Trace;
    use chrono::Duration;
    use std::path::PathBuf;

    /// The join, end to end, against an export written to a temporary
    /// directory — `Archive` reads a directory and a Zip through the same
    /// path, and a directory is the one a failing test can be read by hand.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root =
                std::env::temp_dir().join(format!("pathify-takeout-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for (path, contents) in files {
                let full = root.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, contents).unwrap();
            }
            Self(root)
        }

        fn open(&self) -> Archive {
            Archive::open(std::slice::from_ref(&self.0)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const HEALTH: &str = "Takeout/Google Health";

    fn exercise_logs() -> String {
        r#"[
          {"logId": 1, "activityName": "Walk", "startTime": "07/11/26 11:30:00",
           "activeDuration": 600000, "hasGps": true,
           "tcxLink": "https://www.fitbit.com/x?export=tcx"},
          {"logId": 2, "activityName": "Outdoor Bike", "startTime": "07/11/26 23:55:00",
           "activeDuration": 600000, "hasGps": true},
          {"logId": 3, "activityName": "Swim", "startTime": "07/20/26 09:00:00",
           "activeDuration": 600000, "hasGps": true},
          {"logId": 4, "activityName": "Walk", "startTime": "07/13/26 08:00:00",
           "activeDuration": 600000, "hasGps": false}
        ]"#
        .to_string()
    }

    /// Two devices interleaved, three unrelated outings, and a six-minute
    /// hole in the middle of the first one.
    const DAY_ONE: &str = "timestamp,latitude,longitude,altitude,data source\n\
        2026-07-11T09:00:00Z,41.39,-81.75,230.0,Phone\n\
        2026-07-11T11:30:00Z,41.3931,-81.7433,236.0,Phone\n\
        2026-07-11T11:30:01.250Z,41.3932,-81.7434,236.5,Watch\n\
        2026-07-11T11:31:00Z,41.3936,-81.7439,236.4,Phone\n\
        2026-07-11T11:31:01Z,41.3937,-81.7440,236.9,Watch\n\
        2026-07-11T11:38:00Z,41.3971,-81.7481,238.0,Phone\n\
        2026-07-11T11:39:00Z,41.3976,-81.7487,238.4,Phone\n\
        2026-07-11T11:45:00Z,41.3990,-81.7500,239.0,Phone\n\
        2026-07-11T23:58:00Z,41.4110,-81.7612,240.4,Phone\n\
        2026-07-11T23:59:00Z,41.4115,-81.7618,240.6,Phone\n";

    const DAY_TWO: &str = "timestamp,latitude,longitude,altitude,data source\n\
        2026-07-12T00:00:00Z,41.4120,-81.7624,240.8,Phone\n\
        2026-07-12T00:01:00Z,41.4125,-81.7630,241.0,Phone\n";

    fn fixture(name: &str) -> Fixture {
        Fixture::new(
            name,
            &[
                (
                    &format!("{HEALTH}/Global Export Data/exercise-0.json"),
                    &exercise_logs(),
                ),
                (
                    &format!("{HEALTH}/Physical Activity_GoogleData/gps_location_2026-07-11.csv"),
                    DAY_ONE,
                ),
                (
                    &format!("{HEALTH}/Physical Activity_GoogleData/gps_location_2026-07-12.csv"),
                    DAY_TWO,
                ),
                (
                    &format!("{HEALTH}/Physical Activity_GoogleData/gps_location_readme.txt"),
                    "not a day file",
                ),
                (
                    &format!("{HEALTH}/Menstrual Health/menstrual_health_settings.csv"),
                    "setting,value\ncycle_length,28\n",
                ),
            ],
        )
    }

    fn options(types: &[&str]) -> Options {
        Options {
            types: types.iter().map(|t| t.to_string()).collect(),
            segment_gap: Duration::seconds(120),
            ..Options::default()
        }
    }

    fn run(archive: &mut Archive, options: &Options) -> (Trace, Report) {
        let catalog = Catalog::of(archive).unwrap();
        extract(archive, &catalog, options).unwrap()
    }

    #[test]
    fn the_backend_is_detected_from_the_entry_names() {
        let fixture = fixture("backend");
        assert_eq!(fixture.open().backend(), Some(Backend::Fitbit));
    }

    #[test]
    fn an_export_with_no_location_data_names_what_it_does_hold() {
        let fixture = Fixture::new(
            "empty",
            &[("Takeout/YouTube and YouTube Music/history/watch.json", "[]")],
        );
        let archive = fixture.open();
        assert_eq!(archive.backend(), None);
        assert_eq!(
            archive.products(),
            vec!["Takeout/YouTube and YouTube Music".to_string()]
        );
    }

    /// A day file is a day, not an activity: the walk is ten minutes out of a
    /// file that also holds a morning errand and an evening ride.
    #[test]
    fn an_activity_is_sliced_out_of_its_day_file() {
        let fixture = fixture("slice");
        let mut archive = fixture.open();
        let (trace, report) = run(&mut archive, &options(&["walk"]));

        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.tracks[0].name.as_deref(), Some("Walk 2026-07-11"));
        assert_eq!(trace.tracks[0].description.as_deref(), Some("Fitbit log 1"));
        // Four of the ten rows in the file: the ones inside the window and
        // recorded by the device that saw the most of it.
        assert_eq!(trace.point_count(), 4);
        assert_eq!(report.selected, 1);
        assert!(report.offset_detected);
        assert_eq!(report.offset_s, 0);
    }

    /// Never a concatenation of the two devices: they are the same journey,
    /// and stringing them together doubles the distance.
    #[test]
    fn only_one_recording_device_survives() {
        let fixture = fixture("sources");
        let mut archive = fixture.open();

        let (trace, report) = run(&mut archive, &options(&["walk"]));
        assert_eq!(
            report.source_choices,
            vec![(1, "Phone".to_string(), vec!["Watch".to_string()])]
        );
        assert_eq!(trace.point_count(), 4);
        // The Phone's first fix, not the Watch's a second later.
        assert_eq!(trace.points().next().unwrap().elevation, Some(236.0));

        let mut named = options(&["walk"]);
        named.source = Some("watch".to_string());
        let (trace, _) = run(&mut archive, &named);
        assert_eq!(trace.point_count(), 2);
        assert_eq!(trace.points().next().unwrap().elevation, Some(236.5));
    }

    #[test]
    fn naming_a_device_that_never_recorded_is_an_error() {
        let fixture = fixture("badsource");
        let mut archive = fixture.open();
        let mut named = options(&["walk"]);
        named.source = Some("Garmin".to_string());
        let catalog = Catalog::of(&mut archive).unwrap();
        let message = extract(&mut archive, &catalog, &named)
            .unwrap_err()
            .to_string();
        assert!(message.contains("Garmin"), "{message}");
        assert!(message.contains("Phone"), "{message}");
    }

    /// The six-minute hole is a pause, not six minutes of walking in a
    /// straight line, so it becomes a segment boundary.
    #[test]
    fn a_gap_inside_the_window_starts_a_new_segment() {
        let fixture = fixture("gap");
        let mut archive = fixture.open();
        let (trace, _) = run(&mut archive, &options(&["walk"]));
        assert_eq!(trace.segment_count(), 2);
        assert_eq!(trace.tracks[0].segments[0].len(), 2);
        assert_eq!(trace.tracks[0].segments[1].len(), 2);

        // Widen the gap past the hole and it is one segment again.
        let mut wide = options(&["walk"]);
        wide.segment_gap = Duration::minutes(10);
        let (trace, _) = run(&mut archive, &wide);
        assert_eq!(trace.segment_count(), 1);
    }

    /// A ride that runs past midnight UTC lives in two day files. Reading
    /// only the first would cut it off at the date line.
    #[test]
    fn a_window_across_midnight_is_stitched_from_both_days() {
        let fixture = fixture("midnight");
        let mut archive = fixture.open();
        let (trace, _) = run(&mut archive, &options(&["outdoor bike"]));

        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.point_count(), 4);
        assert_eq!(trace.segment_count(), 1, "one minute apart is not a pause");
        let (start, end) = trace.time_range().unwrap();
        assert_eq!(start.to_rfc3339(), "2026-07-11T23:58:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-07-12T00:01:00+00:00");
    }

    /// A log can claim GPS with no day file behind it. Saying so is the
    /// point: a silently short trace looks like a complete one.
    #[test]
    fn a_log_with_no_day_file_is_reported_rather_than_dropped() {
        let fixture = fixture("missing");
        let mut archive = fixture.open();
        let (trace, report) = run(&mut archive, &options(&["swim"]));

        assert!(trace.is_empty());
        assert_eq!(report.selected, 1);
        assert_eq!(report.tracks, 0);
        assert_eq!(
            report.missing_days,
            vec![(3, chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap())]
        );
    }

    /// Half a ride is not a short ride. A window that straddles midnight
    /// with only one of its two day files present has to say so.
    #[test]
    fn a_window_half_covered_by_the_archive_reports_the_missing_half() {
        let fixture = Fixture::new(
            "halfnight",
            &[
                (
                    &format!("{HEALTH}/Global Export Data/exercise-0.json"),
                    &exercise_logs(),
                ),
                (
                    &format!("{HEALTH}/Physical Activity_GoogleData/gps_location_2026-07-11.csv"),
                    DAY_ONE,
                ),
            ],
        );
        let mut archive = fixture.open();
        // Pinned rather than detected: with one activity and one day file to
        // go on there is not enough evidence to measure the offset from, and
        // this test is about the report, not about the measurement.
        let mut pinned = options(&["outdoor bike"]);
        pinned.offset = Some(Duration::zero());
        let (trace, report) = run(&mut archive, &pinned);

        assert_eq!(trace.point_count(), 2, "only the 07-11 half exists");
        assert_eq!(
            report.missing_days,
            vec![(2, chrono::NaiveDate::from_ymd_opt(2026, 7, 12).unwrap())]
        );
        assert!(report.empty_windows.is_empty());
    }

    /// Every type at once, which is also what an empty filter means.
    #[test]
    fn tracks_come_out_in_start_order() {
        let fixture = fixture("order");
        let mut archive = fixture.open();
        let (trace, report) = run(&mut archive, &options(&[]));

        let names: Vec<&str> = trace
            .tracks
            .iter()
            .filter_map(|t| t.name.as_deref())
            .collect();
        assert_eq!(names, vec!["Walk 2026-07-11", "Outdoor Bike 2026-07-11"]);
        // The fourth log has no GPS, so it was never a candidate.
        assert_eq!(report.selected, 3);
        assert_eq!(report.logs, 4);
        assert_eq!(report.logs_with_gps, 3);
    }

    /// The same join, packaged one trace per activity: a caller importing
    /// each outing as its own record must not have to carve the combined
    /// trace back up to find where one ended and the next began.
    #[test]
    fn each_activity_can_come_out_as_its_own_trace() {
        let fixture = fixture("each");
        let mut archive = fixture.open();
        let catalog = Catalog::of(&mut archive).unwrap();
        let options = options(&[]);
        let (activities, report) = extract_each(&mut archive, &catalog, &options).unwrap();

        assert_eq!(activities.len(), 2);
        assert_eq!(report.tracks, 2);

        let walk = &activities[0];
        assert_eq!(walk.log_id, 1);
        assert_eq!(walk.name, "Walk");
        assert_eq!(walk.start.to_rfc3339(), "2026-07-11T11:30:00+00:00");
        assert_eq!(walk.source, "Phone");
        // One track, and the trace names itself after the activity rather
        // than after the export it was pulled out of.
        assert_eq!(walk.trace.track_count(), 1);
        assert_eq!(walk.trace.metadata.name.as_deref(), Some("Walk 2026-07-11"));
        assert_eq!(
            walk.trace.metadata.description.as_deref(),
            Some("Fitbit log 1")
        );
        assert_eq!(
            walk.trace.metadata.source_device.as_deref(),
            Some("Phone"),
            "a single activity knows which device recorded it"
        );

        // Same points, same order, same report: only the packaging differs.
        let (combined, combined_report) = run(&mut archive, &options);
        assert_eq!(combined_report, report);
        let split: Vec<_> = activities
            .iter()
            .flat_map(|activity| activity.trace.points().cloned())
            .collect();
        assert_eq!(split, combined.points().cloned().collect::<Vec<_>>());
    }

    /// The wall clock in the exercise logs carries no offset. Rather than
    /// assume UTC, the offset that lands the windows on recorded points is
    /// measured — here by shifting the whole export four hours west.
    #[test]
    fn the_wall_clocks_offset_from_utc_is_measured_not_assumed() {
        let shifted = exercise_logs().replace("11:30:00", "07:30:00");
        let fixture = Fixture::new(
            "offset",
            &[
                (
                    &format!("{HEALTH}/Global Export Data/exercise-0.json"),
                    &shifted,
                ),
                (
                    &format!("{HEALTH}/Physical Activity_GoogleData/gps_location_2026-07-11.csv"),
                    DAY_ONE,
                ),
            ],
        );
        let mut archive = fixture.open();
        let (trace, report) = run(&mut archive, &options(&["walk"]));

        assert!(report.offset_detected);
        assert_eq!(report.offset_s, -4 * 3600);
        assert_eq!(
            trace.point_count(),
            4,
            "the same four points, four hours over"
        );
    }

    /// The types menu counts what can actually become a track, and reports
    /// the rest rather than hiding them.
    #[test]
    fn types_report_what_can_and_cannot_produce_a_track() {
        let fixture = fixture("types");
        let mut archive = fixture.open();
        let types = Catalog::of(&mut archive).unwrap().types();

        let walk = types.iter().find(|t| t.name == "Walk").unwrap();
        assert_eq!((walk.with_gps, walk.without_gps), (1, 1));
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn the_readme_beside_the_day_files_is_not_read_as_one() {
        let fixture = fixture("readme");
        let mut archive = fixture.open();
        assert_eq!(Catalog::of(&mut archive).unwrap().day_count(), 2);
    }
}
