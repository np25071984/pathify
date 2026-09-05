//! Plausibility bounds on movement between two fixes.
//!
//! Both drift filtering (`clean`) and point matching (`merge`) need the same
//! underlying question answered — *could a device really have gotten from here
//! to there in this much time?* — so the bound lives in one place rather than
//! being re-tuned separately in each.

use chrono::Duration;

use crate::model::{Point, Segment, Trace, Track};

use super::distance::distance;

/// Fastest speed a GPS trace is expected to imply, in meters per second.
///
/// About 216 km/h: comfortably above any running, cycling, or road-vehicle
/// recording, and low enough that a fix teleporting across a city is caught.
/// A step implying more than this is a bad fix, not travel.
pub const MAX_PLAUSIBLE_SPEED_MPS: f64 = 60.0;

/// How far apart two devices can report the same spot, in meters.
///
/// Consumer GPS under open sky lands within roughly this much of truth, and two
/// receivers at the same place disagree by about as much, so any comparison
/// between recordings has to allow for it before concluding they differ.
pub const GPS_NOISE_MARGIN_M: f64 = 15.0;

/// Speed implied by moving from `a` to `b`, in meters per second.
///
/// `None` when either point lacks a timestamp or the two share one — an
/// instantaneous step has no defined speed, and dividing by zero here would
/// quietly produce an infinity that compares greater than every threshold.
pub fn implied_speed(a: &Point, b: &Point) -> Option<f64> {
    let (start, end) = (a.time?, b.time?);
    let seconds = (end - start).num_milliseconds().abs() as f64 / 1000.0;
    if seconds == 0.0 {
        return None;
    }
    Some(distance(a, b) / seconds)
}

/// Whether the step from `a` to `b` is physically plausible at `max_speed_mps`.
///
/// Points without timestamps are accepted: with no time axis there is no
/// evidence of implausibility, and discarding data on a guess is worse than
/// keeping a suspect fix.
pub fn is_plausible_step(a: &Point, b: &Point, max_speed_mps: f64) -> bool {
    match implied_speed(a, b) {
        Some(speed) => speed <= max_speed_mps,
        None => true,
    }
}

/// How far apart two fixes of the *same moment* may legitimately be, in meters.
///
/// Two devices recording the same activity sample at different instants, so
/// over a gap of `apart` the subject genuinely moved; the GPS noise margin
/// covers the disagreement that remains at zero separation.
pub fn coincidence_radius_m(apart: Duration, max_speed_mps: f64) -> f64 {
    let seconds = apart.num_milliseconds().abs() as f64 / 1000.0;
    max_speed_mps * seconds + GPS_NOISE_MARGIN_M
}

/// How many rejections in a row before the anchor itself is suspect.
///
/// Filtering compares each fix against the last one it kept, which goes wrong
/// if the kept fix was the bad one: every good point after it then looks like a
/// jump, and a whole trace can be deleted defending against a single outlier.
/// After this many consecutive rejections the anchor is treated as the error,
/// not the data.
pub const MAX_CONSECUTIVE_REJECTIONS: usize = 3;

/// Drop fixes that could not have been reached from the previous good fix, and
/// report how many were removed.
///
/// Unlike redaction, this does *not* split segments. A drift outlier is a bad
/// reading of a position the subject really did travel through, so joining its
/// neighbours restores the true path; the traveler was there, the receiver just
/// lied about where.
pub fn filter_drift(trace: &Trace, max_speed_mps: f64) -> (Trace, usize) {
    let mut removed = 0;
    let mut tracks = Vec::new();

    for track in &trace.tracks {
        let mut segments = Vec::new();

        for segment in &track.segments {
            let mut kept: Vec<Point> = Vec::new();
            let mut rejections = 0;

            for point in &segment.points {
                let plausible = match kept.last() {
                    Some(anchor) => is_plausible_step(anchor, point, max_speed_mps),
                    None => true,
                };

                if plausible || rejections >= MAX_CONSECUTIVE_REJECTIONS {
                    if !plausible {
                        // Accepted only because the anchor is the likelier
                        // culprit; take this point as the new reference.
                        rejections = 0;
                    }
                    kept.push(point.clone());
                } else {
                    rejections += 1;
                    removed += 1;
                }
            }

            if !kept.is_empty() {
                segments.push(Segment::new(kept));
            }
        }

        if !segments.is_empty() {
            tracks.push(Track {
                name: track.name.clone(),
                description: track.description.clone(),
                segments,
            });
        }
    }

    (
        Trace {
            metadata: trace.metadata.clone(),
            tracks,
        },
        removed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Metadata;
    use chrono::{TimeZone, Utc};

    fn at(lat: f64, lon: f64, seconds: i64) -> Point {
        Point::new(lat, lon).unwrap().with_time(Some(
            Utc.with_ymd_and_hms(2024, 5, 1, 15, 0, 0).unwrap() + Duration::seconds(seconds),
        ))
    }

    #[test]
    fn computes_speed_from_distance_and_time() {
        // One degree of longitude at the equator is ~111 km; over 3600 s that
        // is ~30.9 m/s.
        let speed = implied_speed(&at(0.0, 0.0, 0), &at(0.0, 1.0, 3600)).unwrap();
        assert!((speed - 30.9).abs() < 0.2, "got {speed}");
    }

    #[test]
    fn speed_is_undefined_without_time_or_across_zero_seconds() {
        let timeless = Point::new(0.0, 0.0).unwrap();
        assert_eq!(implied_speed(&timeless, &at(0.0, 1.0, 10)), None);
        assert_eq!(implied_speed(&at(0.0, 0.0, 5), &at(0.0, 1.0, 5)), None);
    }

    #[test]
    fn speed_is_direction_independent() {
        let (a, b) = (at(0.0, 0.0, 0), at(0.0, 0.1, 60));
        let forward = implied_speed(&a, &b).unwrap();
        let backward = implied_speed(&b, &a).unwrap();
        assert!((forward - backward).abs() < 1e-9);
    }

    #[test]
    fn catches_a_fix_that_teleports() {
        // ~11 km in one second.
        let jump = (at(0.0, 0.0, 0), at(0.0, 0.1, 1));
        assert!(!is_plausible_step(
            &jump.0,
            &jump.1,
            MAX_PLAUSIBLE_SPEED_MPS
        ));

        // ~1.1 m in one second is an ordinary walking pace.
        let walk = (at(0.0, 0.0, 0), at(0.0, 0.00001, 1));
        assert!(is_plausible_step(&walk.0, &walk.1, MAX_PLAUSIBLE_SPEED_MPS));
    }

    /// Without timestamps there is no evidence of a bad fix, and throwing data
    /// away on a guess is the worse failure.
    #[test]
    fn keeps_steps_it_cannot_judge() {
        let a = Point::new(0.0, 0.0).unwrap();
        let b = Point::new(50.0, 50.0).unwrap();
        assert!(is_plausible_step(&a, &b, MAX_PLAUSIBLE_SPEED_MPS));
    }

    #[test]
    fn coincidence_radius_grows_with_the_gap() {
        let immediate = coincidence_radius_m(Duration::zero(), MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(immediate, GPS_NOISE_MARGIN_M);

        let two_seconds = coincidence_radius_m(Duration::seconds(2), MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(
            two_seconds,
            2.0 * MAX_PLAUSIBLE_SPEED_MPS + GPS_NOISE_MARGIN_M
        );

        // Sign of the gap must not matter.
        assert_eq!(
            coincidence_radius_m(Duration::seconds(-2), MAX_PLAUSIBLE_SPEED_MPS),
            two_seconds
        );
    }

    fn trace_of(points: Vec<Point>) -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![Segment::new(points)],
            }],
        }
    }

    /// A walking pace with one fix that teleports across the city and back.
    #[test]
    fn removes_a_single_bad_fix_and_keeps_the_rest() {
        let trace = trace_of(vec![
            at(47.6000, -122.3300, 0),
            at(47.6001, -122.3300, 10),
            at(47.9000, -122.3300, 11), // ~33 km in one second
            at(47.6002, -122.3300, 20),
            at(47.6003, -122.3300, 30),
        ]);

        let (cleaned, removed) = filter_drift(&trace, MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(removed, 1);
        assert_eq!(cleaned.point_count(), 4);
        assert!(
            cleaned.points().all(|p| p.lat < 47.7),
            "the outlier survived"
        );
    }

    /// Drift removal joins the outlier's neighbours instead of splitting: the
    /// subject really did travel through there, the receiver just lied.
    #[test]
    fn filtering_does_not_split_the_segment() {
        let trace = trace_of(vec![
            at(47.6000, -122.3300, 0),
            at(47.9000, -122.3300, 1),
            at(47.6002, -122.3300, 20),
        ]);
        let (cleaned, _) = filter_drift(&trace, MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(cleaned.segment_count(), 1);
    }

    #[test]
    fn a_clean_trace_is_left_alone() {
        let trace = trace_of(vec![
            at(47.6000, -122.3300, 0),
            at(47.6001, -122.3300, 10),
            at(47.6002, -122.3300, 20),
        ]);
        let (cleaned, removed) = filter_drift(&trace, MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(removed, 0);
        assert_eq!(cleaned, trace);
    }

    /// The failure this guards against: if the *first* fix is the bad one,
    /// naive filtering measures every good point against it and deletes the
    /// whole recording.
    #[test]
    fn a_bad_first_fix_does_not_delete_the_whole_trace() {
        let mut points = vec![at(47.9000, -122.3300, 0)]; // nowhere near the rest
        for i in 1..10 {
            points.push(at(47.6000 + i as f64 * 0.0001, -122.3300, i * 10));
        }

        let (cleaned, removed) = filter_drift(&trace_of(points), MAX_PLAUSIBLE_SPEED_MPS);
        assert!(
            cleaned.point_count() >= 6,
            "kept only {} points; the anchor heuristic did not recover",
            cleaned.point_count()
        );
        assert!(removed <= MAX_CONSECUTIVE_REJECTIONS + 1);
    }

    #[test]
    fn points_without_timestamps_are_never_dropped() {
        let trace = trace_of(vec![
            Point::new(47.6000, -122.3300).unwrap(),
            Point::new(47.9000, -122.3300).unwrap(),
            Point::new(47.6002, -122.3300).unwrap(),
        ]);
        let (cleaned, removed) = filter_drift(&trace, MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(removed, 0);
        assert_eq!(cleaned.point_count(), 3);
    }

    #[test]
    fn an_empty_trace_survives_filtering() {
        let (cleaned, removed) = filter_drift(&Trace::default(), MAX_PLAUSIBLE_SPEED_MPS);
        assert_eq!(removed, 0);
        assert!(cleaned.is_empty());
    }
}
