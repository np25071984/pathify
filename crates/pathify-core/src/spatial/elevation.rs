use serde::Serialize;

use crate::model::Trace;

/// Default hysteresis threshold, in meters.
///
/// Consumer GPS elevation jitters by a couple of meters even standing still, so
/// summing every raw delta reports hundreds of meters of "gain" on a flat ride.
/// Only counting movement that exceeds this threshold is what makes the number
/// mean anything.
pub const DEFAULT_NOISE_THRESHOLD_M: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ElevationStats {
    /// Total climb in meters, after noise filtering.
    pub gain_m: f64,
    /// Total descent in meters, after noise filtering.
    pub loss_m: f64,
    pub min_m: f64,
    pub max_m: f64,
    /// Threshold the gain and loss figures were computed with.
    pub threshold_m: f64,
}

/// Elevation gain, loss, and range for a trace, or `None` when no point in it
/// carries an elevation.
///
/// Gain and loss use hysteresis rather than a per-sample delta sum: a running
/// reference only moves once the elevation has departed from it by more than
/// `threshold_m`, so jitter below that never accumulates.
pub fn elevation_stats(trace: &Trace, threshold_m: f64) -> Option<ElevationStats> {
    let mut gain = 0.0;
    let mut loss = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut saw_any = false;

    for segment in trace.segments() {
        // The reference resets per segment: across a pause the recording may
        // resume somewhere else entirely, and that step is not climb.
        let mut reference: Option<f64> = None;

        for elevation in segment.points.iter().filter_map(|p| p.elevation) {
            saw_any = true;
            min = min.min(elevation);
            max = max.max(elevation);

            match reference {
                None => reference = Some(elevation),
                Some(reference_m) => {
                    let delta = elevation - reference_m;
                    if delta > threshold_m {
                        gain += delta;
                        reference = Some(elevation);
                    } else if -delta > threshold_m {
                        loss += -delta;
                        reference = Some(elevation);
                    }
                }
            }
        }
    }

    saw_any.then_some(ElevationStats {
        gain_m: gain,
        loss_m: loss,
        min_m: min,
        max_m: max,
        threshold_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Point, Segment, Track};

    fn trace_from(segments: Vec<Vec<Option<f64>>>) -> Trace {
        Trace {
            metadata: Default::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: segments
                    .into_iter()
                    .map(|elevations| {
                        Segment::new(
                            elevations
                                .into_iter()
                                .enumerate()
                                .map(|(i, e)| {
                                    Point::new(0.0, i as f64 * 0.001).unwrap().with_elevation(e)
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn none_when_no_elevation_recorded() {
        assert!(elevation_stats(&trace_from(vec![vec![None, None]]), 3.0).is_none());
        assert!(elevation_stats(&Trace::default(), 3.0).is_none());
    }

    #[test]
    fn sums_climb_and_descent_above_threshold() {
        let trace = trace_from(vec![vec![
            Some(100.0),
            Some(150.0),
            Some(120.0),
            Some(170.0),
        ]]);
        let stats = elevation_stats(&trace, 3.0).unwrap();
        assert_eq!(stats.gain_m, 100.0); // +50 then +50
        assert_eq!(stats.loss_m, 30.0);
        assert_eq!(stats.min_m, 100.0);
        assert_eq!(stats.max_m, 170.0);
    }

    /// The whole point of the threshold: a flat recording that jitters by a
    /// meter or two must report zero gain, not a slow accumulation.
    #[test]
    fn ignores_jitter_below_threshold() {
        let trace = trace_from(vec![vec![
            Some(100.0),
            Some(101.5),
            Some(99.0),
            Some(101.0),
            Some(100.5),
            Some(99.5),
        ]]);
        let stats = elevation_stats(&trace, 3.0).unwrap();
        assert_eq!(stats.gain_m, 0.0);
        assert_eq!(stats.loss_m, 0.0);
        // Range still reflects the raw readings.
        assert_eq!(stats.min_m, 99.0);
        assert_eq!(stats.max_m, 101.5);
    }

    #[test]
    fn a_zero_threshold_sums_every_delta() {
        let trace = trace_from(vec![vec![Some(100.0), Some(101.0), Some(100.0)]]);
        let stats = elevation_stats(&trace, 0.0).unwrap();
        assert_eq!(stats.gain_m, 1.0);
        assert_eq!(stats.loss_m, 1.0);
    }

    /// A jump across a segment boundary is a gap in the recording, not a climb.
    #[test]
    fn does_not_count_climb_across_a_segment_boundary() {
        let trace = trace_from(vec![vec![Some(100.0)], vec![Some(900.0)]]);
        let stats = elevation_stats(&trace, 3.0).unwrap();
        assert_eq!(stats.gain_m, 0.0);
        assert_eq!(stats.max_m, 900.0);
    }

    #[test]
    fn skips_points_missing_elevation() {
        let trace = trace_from(vec![vec![Some(100.0), None, Some(150.0)]]);
        let stats = elevation_stats(&trace, 3.0).unwrap();
        assert_eq!(stats.gain_m, 50.0);
    }
}
