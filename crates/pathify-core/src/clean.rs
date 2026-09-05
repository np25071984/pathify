//! Removing what should not be in a trace: bad fixes, and private locations.
//!
//! The two removals are deliberately different in kind, and the order matters.
//! Drift filtering discards *readings* the receiver got wrong, so it runs first
//! and stitches its neighbours back together. Redaction discards *real*
//! locations the user chose not to share, so it runs afterwards on the tidied
//! path and leaves an honest gap where it cut.

use serde::Serialize;

use crate::model::Trace;
use crate::spatial::drift::{MAX_PLAUSIBLE_SPEED_MPS, filter_drift};
use crate::spatial::privacy::{Geofence, redact, trim_ends};

#[derive(Debug, Clone)]
pub struct CleanOptions {
    /// Remove fixes that imply impossible speeds.
    pub drift: bool,
    /// Speed above which a step is treated as a bad fix, in meters per second.
    pub max_speed_mps: f64,
    /// Areas whose points must not appear in the output.
    pub fences: Vec<Geofence>,
    /// Remove everything within this many meters of where the trace starts and
    /// ends.
    pub trim_ends_m: Option<f64>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            // Drift filtering is on by default because it only ever removes
            // readings that are physically impossible. Redaction is not: it
            // deletes real locations, and which ones is the user's to say.
            drift: true,
            max_speed_mps: MAX_PLAUSIBLE_SPEED_MPS,
            fences: Vec::new(),
            trim_ends_m: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CleanReport {
    pub points_before: usize,
    pub points_after: usize,
    /// Fixes discarded as physically impossible.
    pub drift_removed: usize,
    /// Points removed by an explicit geofence.
    pub redacted_removed: usize,
    /// Points removed for being near the start or end of the journey.
    pub trimmed_removed: usize,
}

impl CleanReport {
    /// Whether any location was removed for privacy, as opposed to for being a
    /// bad reading.
    pub fn redacted_anything(&self) -> bool {
        self.redacted_removed + self.trimmed_removed > 0
    }
}

/// Filter drift and apply redaction, reporting what was removed.
pub fn clean(trace: &Trace, options: &CleanOptions) -> (Trace, CleanReport) {
    let points_before = trace.point_count();

    let (mut current, drift_removed) = if options.drift {
        filter_drift(trace, options.max_speed_mps)
    } else {
        (trace.clone(), 0)
    };

    let mut redacted_removed = 0;
    if !options.fences.is_empty() {
        let (next, removed) = redact(&current, &options.fences);
        current = next;
        redacted_removed = removed;
    }

    let mut trimmed_removed = 0;
    if let Some(radius_m) = options.trim_ends_m {
        // Trimming runs last so it anchors on the endpoints of the path the
        // user will actually publish, not on a drift outlier that is about to
        // be discarded anyway.
        let (next, removed) = trim_ends(&current, radius_m);
        current = next;
        trimmed_removed = removed;
    }

    let points_after = current.point_count();
    (
        current,
        CleanReport {
            points_before,
            points_after,
            drift_removed,
            redacted_removed,
            trimmed_removed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Metadata, Point, Segment, Track};
    use crate::spatial::distance::distance;
    use chrono::{Duration, TimeZone, Utc};

    fn at(lat: f64, lon: f64, seconds: i64) -> Point {
        Point::new(lat, lon).unwrap().with_time(Some(
            Utc.with_ymd_and_hms(2024, 5, 1, 15, 0, 0).unwrap() + Duration::seconds(seconds),
        ))
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

    /// A walk north from home, with one fix that teleports.
    fn sample() -> Trace {
        trace_of(vec![
            at(47.6000, -122.3300, 0),
            at(47.6010, -122.3300, 60),
            at(47.9000, -122.3300, 61), // impossible
            at(47.6020, -122.3300, 120),
            at(47.6030, -122.3300, 180),
            at(47.6040, -122.3300, 240),
        ])
    }

    #[test]
    fn drift_filtering_is_on_by_default() {
        let (cleaned, report) = clean(&sample(), &CleanOptions::default());
        assert_eq!(report.drift_removed, 1);
        assert_eq!(report.points_before, 6);
        assert_eq!(report.points_after, 5);
        assert!(cleaned.points().all(|p| p.lat < 47.7));
    }

    /// Redaction must never happen unless it was asked for — silently deleting
    /// locations would be its own kind of bug.
    #[test]
    fn nothing_is_redacted_without_being_asked() {
        let (_, report) = clean(&sample(), &CleanOptions::default());
        assert_eq!(report.redacted_removed, 0);
        assert_eq!(report.trimmed_removed, 0);
        assert!(!report.redacted_anything());
    }

    #[test]
    fn drift_filtering_can_be_turned_off() {
        let options = CleanOptions {
            drift: false,
            ..CleanOptions::default()
        };
        let (cleaned, report) = clean(&sample(), &options);
        assert_eq!(report.drift_removed, 0);
        assert_eq!(cleaned.point_count(), 6);
    }

    #[test]
    fn an_explicit_fence_removes_its_points() {
        let home = at(47.6000, -122.3300, 0);
        let options = CleanOptions {
            fences: vec![Geofence::new(home.clone(), 200.0)],
            ..CleanOptions::default()
        };

        let (cleaned, report) = clean(&sample(), &options);
        assert!(report.redacted_removed > 0);
        assert!(
            cleaned.points().all(|p| distance(&home, p) > 200.0),
            "a fenced point survived"
        );
    }

    #[test]
    fn trimming_hides_both_ends() {
        let options = CleanOptions {
            trim_ends_m: Some(200.0),
            ..CleanOptions::default()
        };
        let original = sample();
        let start = original.points().next().unwrap().clone();

        let (cleaned, report) = clean(&original, &options);
        assert!(report.trimmed_removed > 0);
        assert!(cleaned.points().all(|p| distance(&start, p) > 200.0));
    }

    /// Trimming anchors on the endpoints of the path the user will publish. If
    /// it ran before drift filtering, a bad final fix would anchor the trim
    /// somewhere the user never was — hiding the wrong place and leaving the
    /// real endpoint exposed.
    #[test]
    fn trimming_anchors_after_drift_is_removed() {
        let trace = trace_of(vec![
            at(47.6000, -122.3300, 0),
            at(47.6010, -122.3300, 60),
            at(47.6020, -122.3300, 120),
            at(47.9000, -122.3300, 121), // bad final fix, far to the north
        ]);

        let real_end = at(47.6020, -122.3300, 120);
        let options = CleanOptions {
            trim_ends_m: Some(150.0),
            ..CleanOptions::default()
        };
        let (cleaned, _) = clean(&trace, &options);

        assert!(
            cleaned.points().all(|p| distance(&real_end, p) > 150.0),
            "the true endpoint was left exposed because the trim anchored on drift"
        );
    }

    #[test]
    fn both_redactions_can_apply_together() {
        let midpoint = at(47.6020, -122.3300, 120);
        let options = CleanOptions {
            fences: vec![Geofence::new(midpoint.clone(), 120.0)],
            trim_ends_m: Some(150.0),
            ..CleanOptions::default()
        };

        let (cleaned, report) = clean(&sample(), &options);
        assert!(report.redacted_removed > 0);
        assert!(report.trimmed_removed > 0);
        assert!(cleaned.points().all(|p| distance(&midpoint, p) > 120.0));
    }

    #[test]
    fn counts_add_up() {
        let options = CleanOptions {
            trim_ends_m: Some(150.0),
            ..CleanOptions::default()
        };
        let (_, report) = clean(&sample(), &options);
        assert_eq!(
            report.points_before - report.points_after,
            report.drift_removed + report.redacted_removed + report.trimmed_removed
        );
    }

    #[test]
    fn cleaning_an_empty_trace_is_harmless() {
        let options = CleanOptions {
            trim_ends_m: Some(100.0),
            ..CleanOptions::default()
        };
        let (cleaned, report) = clean(&Trace::default(), &options);
        assert!(cleaned.is_empty());
        assert_eq!(report.points_before, 0);
        assert_eq!(report.points_after, 0);
    }
}
