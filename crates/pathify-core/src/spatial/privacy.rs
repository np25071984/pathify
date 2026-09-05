//! Removing locations the user does not want in a shared file.
//!
//! This module gets more test rigor than anything else in the crate, because
//! its failure mode is not a wrong number on a screen — a point that should
//! have been removed and was not is a home address in a file someone published.
//! Every function here is written so the guarantee can be stated as an
//! invariant and checked directly: *no surviving point lies inside a fence*.

use crate::model::{Point, Segment, Trace, Track};

use super::distance::distance;

/// A circular area whose points must not appear in the output.
#[derive(Debug, Clone, PartialEq)]
pub struct Geofence {
    pub center: Point,
    pub radius_m: f64,
}

impl Geofence {
    pub fn new(center: Point, radius_m: f64) -> Self {
        Self { center, radius_m }
    }

    /// Whether `point` falls inside the fence.
    ///
    /// The boundary counts as inside: on a privacy control, the inclusive
    /// reading is the safe one.
    pub fn contains(&self, point: &Point) -> bool {
        distance(&self.center, point) <= self.radius_m
    }
}

/// Remove every point inside any fence, and report how many were removed.
///
/// Where points are removed from the middle of a segment, the segment is
/// **split** rather than closed up. Joining the surviving neighbours would draw
/// a straight line through the redacted area — inventing travel that never
/// happened, and handing back a chord that points straight at what was hidden.
/// An honest gap is both more accurate and more private.
pub fn redact(trace: &Trace, fences: &[Geofence]) -> (Trace, usize) {
    if fences.is_empty() {
        return (trace.clone(), 0);
    }

    let mut removed = 0;
    let mut tracks = Vec::new();

    for track in &trace.tracks {
        let mut segments: Vec<Segment> = Vec::new();

        for segment in &track.segments {
            let mut run: Vec<Point> = Vec::new();
            for point in &segment.points {
                if fences.iter().any(|fence| fence.contains(point)) {
                    removed += 1;
                    // The run ends here; what follows starts a new segment.
                    if !run.is_empty() {
                        segments.push(Segment::new(std::mem::take(&mut run)));
                    }
                } else {
                    run.push(point.clone());
                }
            }
            if !run.is_empty() {
                segments.push(Segment::new(run));
            }
        }

        // A track emptied by redaction is dropped rather than kept as a husk.
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

/// Remove everything within `radius_m` of where the trace starts and ends.
///
/// This is the redaction that needs no configuration: it hides the addresses at
/// either end of a journey without the user having to type their home
/// coordinates into a shell command.
///
/// It fences *both* endpoints and removes matching points wherever they occur,
/// not merely leading and trailing runs — a loop that passes the front door
/// halfway round would otherwise reveal exactly what the trim was meant to
/// hide.
pub fn trim_ends(trace: &Trace, radius_m: f64) -> (Trace, usize) {
    let mut points = trace.points();
    let Some(first) = points.next().cloned() else {
        return (trace.clone(), 0);
    };
    let last = trace.points().next_back().cloned().unwrap_or(first.clone());

    redact(
        trace,
        &[
            Geofence::new(first, radius_m),
            Geofence::new(last, radius_m),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Metadata;

    fn p(lat: f64, lon: f64) -> Point {
        Point::new(lat, lon).unwrap()
    }

    fn trace_of(segments: Vec<Vec<Point>>) -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: Some("ride".into()),
                description: None,
                segments: segments.into_iter().map(Segment::new).collect(),
            }],
        }
    }

    /// A line heading due north from home, one point roughly every 111 m.
    fn northward(count: usize) -> Trace {
        trace_of(vec![
            (0..count)
                .map(|i| p(47.60 + i as f64 * 0.001, -122.33))
                .collect(),
        ])
    }

    #[test]
    fn fence_membership_includes_the_boundary() {
        let fence = Geofence::new(p(47.60, -122.33), 200.0);
        assert!(fence.contains(&p(47.60, -122.33)));
        assert!(fence.contains(&p(47.6010, -122.33))); // ~111 m
        assert!(!fence.contains(&p(47.6030, -122.33))); // ~333 m
    }

    /// The guarantee, stated directly: nothing inside the fence survives.
    #[test]
    fn no_surviving_point_lies_inside_a_fence() {
        let fence = Geofence::new(p(47.60, -122.33), 250.0);
        let (cleaned, removed) = redact(&northward(10), std::slice::from_ref(&fence));

        assert!(removed > 0);
        assert!(
            cleaned.points().all(|point| !fence.contains(point)),
            "a redacted point survived"
        );
    }

    #[test]
    fn redaction_splits_a_segment_rather_than_closing_the_gap() {
        // Fence the middle of a single run of points.
        let fence = Geofence::new(p(47.605, -122.33), 150.0);
        let (cleaned, removed) = redact(&northward(11), &[fence]);

        assert_eq!(removed, 3, "the middle points fall inside");
        assert_eq!(
            cleaned.segment_count(),
            2,
            "the survivors must not be joined across the hole"
        );
    }

    #[test]
    fn redaction_at_an_end_does_not_create_an_empty_segment() {
        let fence = Geofence::new(p(47.60, -122.33), 150.0);
        let (cleaned, _) = redact(&northward(10), &[fence]);
        assert_eq!(cleaned.segment_count(), 1);
        assert!(cleaned.segments().all(|s| !s.is_empty()));
    }

    #[test]
    fn a_track_emptied_by_redaction_is_dropped() {
        let fence = Geofence::new(p(47.60, -122.33), 100_000.0);
        let (cleaned, removed) = redact(&northward(5), &[fence]);
        assert_eq!(removed, 5);
        assert_eq!(cleaned.track_count(), 0);
        assert!(cleaned.is_empty());
    }

    #[test]
    fn multiple_fences_all_apply() {
        let fences = vec![
            Geofence::new(p(47.600, -122.33), 150.0),
            Geofence::new(p(47.009, -122.33), 150.0), // matches nothing
            Geofence::new(p(47.610, -122.33), 150.0),
        ];
        let (cleaned, _) = redact(&northward(11), &fences);
        assert!(
            cleaned
                .points()
                .all(|point| !fences.iter().any(|f| f.contains(point)))
        );
    }

    #[test]
    fn no_fences_is_a_no_op() {
        let original = northward(5);
        let (cleaned, removed) = redact(&original, &[]);
        assert_eq!(removed, 0);
        assert_eq!(cleaned, original);
    }

    #[test]
    fn trimming_hides_both_endpoints() {
        let trace = northward(11);
        let start = trace.points().next().unwrap().clone();
        let end = trace.points().next_back().unwrap().clone();

        let (cleaned, removed) = trim_ends(&trace, 250.0);
        assert!(removed > 0);
        assert!(
            cleaned
                .points()
                .all(|point| distance(&start, point) > 250.0)
        );
        assert!(cleaned.points().all(|point| distance(&end, point) > 250.0));
    }

    /// A loop that passes the front door mid-ride must not leak it just because
    /// the pass is neither the first nor the last point.
    #[test]
    fn trimming_also_removes_a_midpoint_pass_near_home() {
        let home = p(47.60, -122.33);
        let trace = trace_of(vec![vec![
            home.clone(),
            p(47.61, -122.33),
            p(47.62, -122.33),
            p(47.6002, -122.33), // back past home, ~22 m away
            p(47.63, -122.33),
            p(47.64, -122.33),
        ]]);

        let (cleaned, _) = trim_ends(&trace, 200.0);
        assert!(
            cleaned.points().all(|point| distance(&home, point) > 200.0),
            "the midpoint pass near home leaked"
        );
    }

    #[test]
    fn trimming_an_empty_trace_is_harmless() {
        let (cleaned, removed) = trim_ends(&Trace::default(), 200.0);
        assert_eq!(removed, 0);
        assert!(cleaned.is_empty());
    }

    #[test]
    fn a_zero_radius_still_removes_the_endpoints_themselves() {
        let (cleaned, removed) = trim_ends(&northward(5), 0.0);
        assert_eq!(removed, 2, "the first and last points are at distance zero");
        assert_eq!(cleaned.point_count(), 3);
    }
}
