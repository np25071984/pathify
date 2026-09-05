//! The metrics `pathify info` reports.
//!
//! Computed in core rather than in the CLI so the numbers are unit-testable
//! without spawning a binary, and so other tools can reuse them.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::formats::Format;
use crate::model::{Bounds, Trace};
use crate::spatial::{ElevationStats, elevation_stats, trace_length};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub source_format: Option<Format>,
    pub name: Option<String>,
    pub tracks: usize,
    pub segments: usize,
    pub points: usize,
    pub distance_m: f64,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    /// Wall-clock span from first to last timestamp. This is elapsed time, not
    /// moving time — a lunch stop counts.
    pub duration_s: Option<i64>,
    /// Distance over elapsed time, in meters per second. `None` without
    /// timestamps, and for a zero-length span.
    pub avg_speed_mps: Option<f64>,
    pub elevation: Option<ElevationStats>,
    pub bounds: Option<Bounds>,
}

impl Summary {
    pub fn of(trace: &Trace, elevation_threshold_m: f64) -> Self {
        let distance_m = trace_length(trace);
        let time_range = trace.time_range();
        let duration_s = time_range.map(|(start, end)| (end - start).num_seconds());
        let avg_speed_mps = duration_s
            .filter(|seconds| *seconds > 0)
            .map(|seconds| distance_m / seconds as f64);

        Self {
            source_format: trace.metadata.source_format,
            name: trace.metadata.name.clone(),
            tracks: trace.track_count(),
            segments: trace.segment_count(),
            points: trace.point_count(),
            distance_m,
            start_time: time_range.map(|(start, _)| start),
            end_time: time_range.map(|(_, end)| end),
            duration_s,
            avg_speed_mps,
            elevation: elevation_stats(trace, elevation_threshold_m),
            bounds: trace.bounds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Point, Segment, Track};
    use chrono::TimeZone;

    fn trace_with_times() -> Trace {
        let start = Utc.with_ymd_and_hms(2024, 5, 1, 15, 0, 0).unwrap();
        Trace {
            metadata: Default::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![Segment::new(vec![
                    Point::new(0.0, 0.0)
                        .unwrap()
                        .with_time(Some(start))
                        .with_elevation(Some(10.0)),
                    Point::new(0.0, 0.01)
                        .unwrap()
                        .with_time(Some(start + chrono::Duration::seconds(100)))
                        .with_elevation(Some(60.0)),
                ])],
            }],
        }
    }

    #[test]
    fn computes_counts_distance_and_duration() {
        let summary = Summary::of(&trace_with_times(), 3.0);
        assert_eq!(summary.points, 2);
        assert_eq!(summary.segments, 1);
        assert_eq!(summary.duration_s, Some(100));
        assert!(summary.distance_m > 1_000.0 && summary.distance_m < 1_200.0);
        let speed = summary.avg_speed_mps.unwrap();
        assert!((speed - summary.distance_m / 100.0).abs() < 1e-9);
        assert_eq!(summary.elevation.unwrap().gain_m, 50.0);
    }

    #[test]
    fn degrades_gracefully_without_timestamps_or_elevation() {
        let trace = Trace {
            metadata: Default::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![Segment::new(vec![
                    Point::new(0.0, 0.0).unwrap(),
                    Point::new(0.0, 0.01).unwrap(),
                ])],
            }],
        };
        let summary = Summary::of(&trace, 3.0);
        assert_eq!(summary.duration_s, None);
        assert_eq!(summary.avg_speed_mps, None);
        assert!(summary.elevation.is_none());
        assert!(summary.distance_m > 0.0);
    }

    #[test]
    fn an_empty_trace_summarizes_to_zeros_not_an_error() {
        let summary = Summary::of(&Trace::default(), 3.0);
        assert_eq!(summary.points, 0);
        assert_eq!(summary.distance_m, 0.0);
        assert!(summary.bounds.is_none());
    }
}
