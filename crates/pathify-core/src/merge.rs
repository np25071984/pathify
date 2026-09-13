//! Combining several traces into one.
//!
//! Two recordings can relate to each other in two quite different ways, and one
//! matching pass cannot serve both:
//!
//! - **Concatenation** — the inputs do not overlap in time. A watch that was
//!   paused and resumed, or two consecutive days. Nothing needs deduplicating;
//!   the recordings just need ordering.
//! - **Reconciliation** — the inputs overlap. A phone and a watch recording the
//!   same ride. Here the same physical moment appears in more than one input and
//!   has to be collapsed, or the merged trace reports double the distance.
//!
//! Which one applies is decided from the inputs' time ranges rather than left
//! to the user to get right.

use chrono::Duration;
use serde::Serialize;

use crate::model::{Metadata, Point, Segment, Trace, Track};
use crate::spatial::dedup::{match_window, median_interval, merge_points, same_moment};

/// Gap that starts a new segment when reconciled points are re-segmented.
///
/// Two minutes, matching the convention GPX writers use for a pause or a loss
/// of signal.
pub const DEFAULT_SEGMENT_GAP: Duration = Duration::seconds(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMode {
    /// Inputs were ordered and stitched; no points were compared.
    Concatenation,
    /// Inputs overlapped in time and were matched point by point.
    Reconciliation,
}

#[derive(Debug, Clone)]
pub struct MergeOptions {
    /// Index of the input that wins on conflict. Defaults to the first.
    pub primary: usize,
    /// Set false to force concatenation even when inputs overlap.
    pub dedup: bool,
    /// Override the derived matching window.
    pub window: Option<Duration>,
    /// Override the speed-derived matching radius, in meters.
    pub radius_m: Option<f64>,
    /// Gap that starts a new segment in reconciled output.
    pub segment_gap: Duration,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            primary: 0,
            dedup: true,
            window: None,
            radius_m: None,
            segment_gap: DEFAULT_SEGMENT_GAP,
        }
    }
}

/// What a merge actually did, for reporting back to the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MergeReport {
    pub mode: MergeMode,
    pub inputs: usize,
    /// Points across all inputs, before merging.
    pub input_points: usize,
    pub output_points: usize,
    /// How many input points were recognized as duplicates of another source's.
    pub matched_points: usize,
    /// The matching window actually used, in seconds.
    pub window_s: Option<f64>,
}

/// Merge traces into one, choosing concatenation or reconciliation by overlap.
pub fn merge(traces: &[Trace], options: &MergeOptions) -> (Trace, MergeReport) {
    let input_points: usize = traces.iter().map(Trace::point_count).sum();

    match traces.len() {
        0 => (
            Trace::default(),
            MergeReport {
                mode: MergeMode::Concatenation,
                inputs: 0,
                input_points: 0,
                output_points: 0,
                matched_points: 0,
                window_s: None,
            },
        ),
        1 => {
            let only = traces[0].clone();
            let output_points = only.point_count();
            (
                only,
                MergeReport {
                    mode: MergeMode::Concatenation,
                    inputs: 1,
                    input_points,
                    output_points,
                    matched_points: 0,
                    window_s: None,
                },
            )
        }
        _ => {
            if options.dedup && any_overlap(traces, options) {
                reconcile(traces, options, input_points)
            } else {
                concatenate(traces, input_points)
            }
        }
    }
}

/// Whether any two inputs cover overlapping spans of time.
///
/// Each span is widened by that pair's matching window before being compared,
/// because bare interval intersection is too strict at the edges: two devices
/// that each recorded a single fix a second apart have spans that do not
/// intersect at all, yet those fixes are plainly the same moment. Widening by
/// exactly the window means "these could contain matchable points".
///
/// A trace with no timestamps anywhere cannot overlap anything — without a time
/// axis there is no way to tell the same moment from a different one, so it is
/// concatenated rather than matched.
fn any_overlap(traces: &[Trace], options: &MergeOptions) -> bool {
    let spans: Vec<_> = traces
        .iter()
        .filter_map(|trace| {
            trace
                .time_range()
                .map(|range| (range, median_interval(trace.points())))
        })
        .collect();

    spans.iter().enumerate().any(|(i, (a, a_interval))| {
        spans[i + 1..].iter().any(|(b, b_interval)| {
            let window = options
                .window
                .unwrap_or_else(|| match_window(*a_interval, *b_interval));
            a.0 - window <= b.1 && b.0 - window <= a.1
        })
    })
}

/// Order the inputs' tracks by start time and emit them unchanged.
///
/// Segment boundaries are preserved exactly: each input already knows where its
/// own pauses were, and nothing here is in a position to second-guess them.
fn concatenate(traces: &[Trace], input_points: usize) -> (Trace, MergeReport) {
    let mut tracks: Vec<(Option<chrono::DateTime<chrono::Utc>>, Track)> = Vec::new();
    for trace in traces {
        for track in &trace.tracks {
            let start = track
                .segments
                .iter()
                .flat_map(|s| &s.points)
                .filter_map(|p| p.time)
                .min();
            tracks.push((start, track.clone()));
        }
    }

    // Undated tracks keep their argument order, after everything datable — a
    // stable sort puts `None` last without disturbing their relative order.
    tracks.sort_by_key(|(start, _)| (start.is_none(), *start));

    let merged = Trace {
        metadata: merged_metadata(traces, 0),
        tracks: tracks.into_iter().map(|(_, track)| track).collect(),
    };
    let output_points = merged.point_count();

    (
        merged,
        MergeReport {
            mode: MergeMode::Concatenation,
            inputs: traces.len(),
            input_points,
            output_points,
            matched_points: 0,
            window_s: None,
        },
    )
}

/// Match overlapping inputs point by point against the primary.
fn reconcile(
    traces: &[Trace],
    options: &MergeOptions,
    input_points: usize,
) -> (Trace, MergeReport) {
    let primary_index = options.primary.min(traces.len() - 1);
    let primary = &traces[primary_index];

    let primary_interval = median_interval(primary.points());
    let mut timeline: Vec<Point> = primary.points().cloned().collect();
    let mut matched_points = 0;
    let mut widest_window = Duration::zero();

    for (index, trace) in traces.iter().enumerate() {
        if index == primary_index {
            continue;
        }

        let window = options
            .window
            .unwrap_or_else(|| match_window(primary_interval, median_interval(trace.points())));
        widest_window = widest_window.max(window);

        // Sorting lets the search look only at the slice inside the window
        // instead of rescanning the whole timeline for every incoming point.
        timeline.sort_by_key(|p| (p.time.is_none(), p.time));

        // A timeline point may absorb at most one point from any single source:
        // two fixes from the same device are two moments, however close.
        let mut claimed = vec![false; timeline.len()];
        let mut unmatched: Vec<Point> = Vec::new();

        for point in trace.points() {
            match best_match(&timeline, &claimed, point, window, options.radius_m) {
                Some(position) => {
                    timeline[position] = merge_points(&timeline[position], point);
                    claimed[position] = true;
                    matched_points += 1;
                }
                None => unmatched.push(point.clone()),
            }
        }

        timeline.extend(unmatched);
    }

    timeline.sort_by_key(|p| (p.time.is_none(), p.time));
    let segments = split_on_gaps(timeline, options.segment_gap);

    let merged = Trace {
        metadata: merged_metadata(traces, primary_index),
        tracks: vec![Track {
            name: primary.tracks.first().and_then(|t| t.name.clone()),
            description: None,
            segments,
        }],
    };
    let output_points = merged.point_count();

    (
        merged,
        MergeReport {
            mode: MergeMode::Reconciliation,
            inputs: traces.len(),
            input_points,
            output_points,
            matched_points,
            window_s: Some(widest_window.num_milliseconds() as f64 / 1000.0),
        },
    )
}

/// Index of the closest-in-time unclaimed point that could be the same moment.
///
/// Nearest in time rather than merely within the window: with a wide window and
/// coarse sampling, several points qualify, and the closest one is the only
/// defensible reading of "the same moment".
fn best_match(
    timeline: &[Point],
    claimed: &[bool],
    point: &Point,
    window: Duration,
    radius_m: Option<f64>,
) -> Option<usize> {
    let time = point.time?;

    // Timeline is sorted by time, so the candidates sit in a contiguous slice.
    let start = timeline.partition_point(|p| match p.time {
        Some(t) => t < time - window,
        None => false,
    });

    let mut best: Option<(usize, Duration)> = None;
    for (offset, candidate) in timeline[start..].iter().enumerate() {
        let position = start + offset;
        match candidate.time {
            // Past the window: everything later is further away still.
            Some(t) if t > time + window => break,
            // Undated points sort last; nothing beyond here can match.
            None => break,
            Some(_) => {}
        }
        if claimed[position] {
            continue;
        }
        if let Some(apart) = same_moment(candidate, point, window, radius_m)
            && best.is_none_or(|(_, closest)| apart < closest)
        {
            best = Some((position, apart));
        }
    }

    best.map(|(position, _)| position)
}

/// Re-derive segment boundaries from gaps in the merged timeline.
///
/// Reconciled output cannot keep any single input's segmentation, because two
/// devices disagree about where the pauses were. Time gaps are the one signal
/// both recordings share.
pub(crate) fn split_on_gaps(points: Vec<Point>, gap: Duration) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut previous: Option<chrono::DateTime<chrono::Utc>> = None;

    for point in points {
        if let (Some(last), Some(now)) = (previous, point.time)
            && now - last > gap
        {
            segments.push(Segment::new(std::mem::take(&mut current)));
        }
        previous = point.time.or(previous);
        current.push(point);
    }
    if !current.is_empty() {
        segments.push(Segment::new(current));
    }
    segments
}

fn merged_metadata(traces: &[Trace], primary: usize) -> Metadata {
    let mut metadata = traces
        .get(primary)
        .map(|t| t.metadata.clone())
        .unwrap_or_default();
    metadata.created = traces
        .iter()
        .filter_map(|t| t.time_range().map(|(start, _)| start))
        .min()
        .or(metadata.created);
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 5, 1, 15, 0, 0).unwrap()
    }

    fn at(lat: f64, lon: f64, seconds: i64) -> Point {
        Point::new(lat, lon)
            .unwrap()
            .with_time(Some(base() + Duration::seconds(seconds)))
    }

    fn trace_of(segments: Vec<Vec<Point>>) -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: segments.into_iter().map(Segment::new).collect(),
            }],
        }
    }

    /// A recording merged with itself must come back unchanged. If matching is
    /// too loose or too tight, this is the first thing that breaks.
    #[test]
    fn merging_a_trace_with_itself_is_idempotent() {
        let trace = trace_of(vec![vec![
            at(47.6062, -122.3321, 0),
            at(47.6070, -122.3330, 10),
            at(47.6080, -122.3340, 20),
        ]]);

        let (merged, report) = merge(&[trace.clone(), trace.clone()], &MergeOptions::default());
        assert_eq!(report.mode, MergeMode::Reconciliation);
        assert_eq!(merged.point_count(), trace.point_count());
        assert_eq!(report.matched_points, 3);

        for (a, b) in trace.points().zip(merged.points()) {
            assert_eq!(a.lat, b.lat);
            assert_eq!(a.lon, b.lon);
            assert_eq!(a.time, b.time);
        }
    }

    #[test]
    fn output_size_is_bounded_by_the_inputs() {
        let a = trace_of(vec![vec![
            at(47.6062, -122.3321, 0),
            at(47.607, -122.333, 10),
        ]]);
        let b = trace_of(vec![vec![
            at(47.6063, -122.3322, 1),
            at(47.6081, -122.3341, 21),
        ]]);

        let (merged, report) = merge(&[a.clone(), b.clone()], &MergeOptions::default());
        let (largest, total) = (
            a.point_count().max(b.point_count()),
            a.point_count() + b.point_count(),
        );
        assert!(merged.point_count() >= largest);
        assert!(merged.point_count() <= total);
        assert_eq!(report.output_points, merged.point_count());
        assert_eq!(report.input_points, total);
    }

    #[test]
    fn a_second_device_contributes_only_its_unmatched_points() {
        let phone = trace_of(vec![vec![
            at(47.6062, -122.3321, 0),
            at(47.6070, -122.3330, 10),
        ]]);
        // Two near-simultaneous duplicates plus one point the phone never saw.
        let watch = trace_of(vec![vec![
            at(47.60621, -122.33211, 1),
            at(47.60701, -122.33301, 11),
            at(47.6090, -122.3350, 40),
        ]]);

        let (merged, report) = merge(&[phone, watch], &MergeOptions::default());
        assert_eq!(report.matched_points, 2);
        assert_eq!(merged.point_count(), 3);
    }

    /// Position must come from the primary, and `--primary` must actually
    /// change which one that is.
    #[test]
    fn the_primary_input_wins_on_position() {
        let a = trace_of(vec![vec![at(47.6062, -122.3321, 0)]]);
        let b = trace_of(vec![vec![at(47.60625, -122.33215, 1)]]);

        let (first_wins, _) = merge(&[a.clone(), b.clone()], &MergeOptions::default());
        assert_eq!(first_wins.points().next().unwrap().lat, 47.6062);

        let options = MergeOptions {
            primary: 1,
            ..MergeOptions::default()
        };
        let (second_wins, _) = merge(&[a, b], &options);
        assert_eq!(second_wins.points().next().unwrap().lat, 47.60625);
    }

    #[test]
    fn non_overlapping_inputs_are_concatenated_in_time_order() {
        let later = trace_of(vec![vec![
            at(47.61, -122.34, 6_000),
            at(47.62, -122.35, 6_030),
        ]]);
        let earlier = trace_of(vec![vec![at(47.60, -122.33, 0), at(47.605, -122.332, 30)]]);

        // Deliberately supplied out of order.
        let (merged, report) = merge(&[later, earlier], &MergeOptions::default());
        assert_eq!(report.mode, MergeMode::Concatenation);
        assert_eq!(merged.point_count(), 4);
        assert_eq!(report.matched_points, 0);

        let times: Vec<_> = merged.points().filter_map(|p| p.time).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    }

    /// Concatenation must not disturb the inputs' own pause boundaries.
    #[test]
    fn concatenation_preserves_segment_structure() {
        let first = trace_of(vec![
            vec![at(47.60, -122.33, 0)],
            vec![at(47.601, -122.331, 30)],
        ]);
        let second = trace_of(vec![vec![at(47.61, -122.34, 9_000)]]);

        let (merged, _) = merge(&[first, second], &MergeOptions::default());
        assert_eq!(merged.segment_count(), 3);
    }

    #[test]
    fn no_dedup_forces_concatenation_even_when_inputs_overlap() {
        let trace = trace_of(vec![vec![at(47.6062, -122.3321, 0)]]);
        let options = MergeOptions {
            dedup: false,
            ..MergeOptions::default()
        };

        let (merged, report) = merge(&[trace.clone(), trace], &options);
        assert_eq!(report.mode, MergeMode::Concatenation);
        assert_eq!(merged.point_count(), 2, "duplicates are kept deliberately");
    }

    /// Without timestamps there is no way to tell the same moment from a
    /// different one, so matching must not be attempted.
    #[test]
    fn inputs_without_timestamps_are_concatenated() {
        let a = trace_of(vec![vec![Point::new(47.6062, -122.3321).unwrap()]]);
        let b = trace_of(vec![vec![Point::new(47.6062, -122.3321).unwrap()]]);

        let (merged, report) = merge(&[a, b], &MergeOptions::default());
        assert_eq!(report.mode, MergeMode::Concatenation);
        assert_eq!(merged.point_count(), 2);
    }

    #[test]
    fn reconciled_output_is_split_on_long_gaps() {
        let a = trace_of(vec![vec![at(47.60, -122.33, 0), at(47.601, -122.331, 10)]]);
        // Overlaps at the start, then resumes after a five-minute break.
        let b = trace_of(vec![vec![at(47.60, -122.33, 1), at(47.61, -122.34, 300)]]);

        let (merged, report) = merge(&[a, b], &MergeOptions::default());
        assert_eq!(report.mode, MergeMode::Reconciliation);
        assert_eq!(merged.segment_count(), 2, "the 290 s gap starts a segment");
    }

    #[test]
    fn a_tight_radius_prevents_a_match() {
        let a = trace_of(vec![vec![at(47.6062, -122.3321, 0)]]);
        // ~11 m apart: inside the default radius (75 m at a 1 s window), so the
        // explicit radius is the only thing that can change the outcome.
        let b = trace_of(vec![vec![at(47.6063, -122.3321, 1)]]);

        let (default_merge, _) = merge(&[a.clone(), b.clone()], &MergeOptions::default());
        assert_eq!(default_merge.point_count(), 1);

        let options = MergeOptions {
            radius_m: Some(5.0),
            ..MergeOptions::default()
        };
        let (tight_merge, report) = merge(&[a, b], &options);
        assert_eq!(tight_merge.point_count(), 2);
        assert_eq!(report.matched_points, 0);
    }

    /// Two fixes from the *same* device are two moments however close together,
    /// so one timeline point must never swallow both.
    #[test]
    fn one_timeline_point_absorbs_at_most_one_point_per_source() {
        let a = trace_of(vec![vec![at(47.6062, -122.3321, 0)]]);
        let b = trace_of(vec![vec![
            at(47.60620, -122.33210, 0),
            at(47.60621, -122.33211, 1),
        ]]);

        let (merged, report) = merge(&[a, b], &MergeOptions::default());
        assert_eq!(report.matched_points, 1);
        assert_eq!(merged.point_count(), 2);
    }

    #[test]
    fn degenerate_input_counts_are_handled() {
        let (empty, report) = merge(&[], &MergeOptions::default());
        assert!(empty.is_empty());
        assert_eq!(report.inputs, 0);

        let single = trace_of(vec![vec![at(47.6062, -122.3321, 0)]]);
        let (passed_through, report) =
            merge(std::slice::from_ref(&single), &MergeOptions::default());
        assert_eq!(passed_through.point_count(), 1);
        assert_eq!(report.inputs, 1);
    }

    #[test]
    fn merging_three_sources_reconciles_all_of_them() {
        let a = trace_of(vec![vec![
            at(47.6062, -122.3321, 0),
            at(47.607, -122.333, 10),
        ]]);
        let b = trace_of(vec![vec![at(47.60621, -122.33211, 1)]]);
        let c = trace_of(vec![vec![at(47.60701, -122.33301, 11)]]);

        let (merged, report) = merge(&[a, b, c], &MergeOptions::default());
        assert_eq!(report.inputs, 3);
        assert_eq!(report.matched_points, 2);
        assert_eq!(merged.point_count(), 2);
    }
}
