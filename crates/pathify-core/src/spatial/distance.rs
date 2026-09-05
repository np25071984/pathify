use geo::{Distance, Geodesic, Point as GeoPoint};

use crate::model::{Point, Segment, Trace};

/// Great-circle distance between two points, in meters.
///
/// Uses a geodesic (ellipsoidal) measure rather than a naive spherical
/// haversine: over a long ride the difference between the two is hundreds of
/// meters, which is exactly the kind of quiet inaccuracy `info` would report
/// without anyone noticing.
pub fn distance(a: &Point, b: &Point) -> f64 {
    Geodesic.distance(to_geo(a), to_geo(b))
}

/// Cumulative length of a segment, in meters.
///
/// Distance is only summed *within* a segment: a segment boundary means the
/// recording paused or lost signal, and connecting across that gap would
/// invent distance the user never traveled.
pub fn segment_length(segment: &Segment) -> f64 {
    segment
        .points
        .windows(2)
        .map(|pair| distance(&pair[0], &pair[1]))
        .sum()
}

/// Total length of every segment in the trace, in meters.
pub fn trace_length(trace: &Trace) -> f64 {
    trace.segments().map(segment_length).sum()
}

fn to_geo(p: &Point) -> GeoPoint<f64> {
    // geo orders coordinates x, y — longitude first.
    GeoPoint::new(p.lon, p.lat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Segment, Track};

    fn p(lat: f64, lon: f64) -> Point {
        Point::new(lat, lon).unwrap()
    }

    /// Reference pair with a widely published geodesic distance: JFK to LHR is
    /// ~5,551 km. Asserting against a known real-world value catches a wrong
    /// formula or a swapped lat/lon far more reliably than a self-consistent
    /// round-trip would.
    #[test]
    fn matches_known_reference_distance() {
        let jfk = p(40.6413, -73.7781);
        let lhr = p(51.4700, -0.4543);
        let meters = distance(&jfk, &lhr);
        assert!(
            (meters - 5_551_000.0).abs() < 5_000.0,
            "expected ~5551 km, got {:.0} m",
            meters
        );
    }

    /// A degree of latitude is ~111.3 km anywhere on the ellipsoid; a degree of
    /// longitude shrinks with the cosine of latitude. Checking both guards
    /// against the classic lat/lon transposition, which a symmetric test misses.
    #[test]
    fn one_degree_of_latitude_is_about_111km() {
        let meters = distance(&p(0.0, 0.0), &p(1.0, 0.0));
        assert!((meters - 110_574.0).abs() < 500.0, "got {meters:.0} m");
    }

    #[test]
    fn longitude_degree_shrinks_toward_the_pole() {
        let at_equator = distance(&p(0.0, 0.0), &p(0.0, 1.0));
        let at_sixty = distance(&p(60.0, 0.0), &p(60.0, 1.0));
        // cos(60°) = 0.5, so the high-latitude degree should be about half.
        assert!((at_sixty / at_equator - 0.5).abs() < 0.01);
    }

    #[test]
    fn distance_is_symmetric_and_zero_to_self() {
        let a = p(47.6062, -122.3321);
        let b = p(45.5152, -122.6784);
        assert_eq!(distance(&a, &a), 0.0);
        assert!((distance(&a, &b) - distance(&b, &a)).abs() < 1e-6);
    }

    #[test]
    fn segment_length_sums_consecutive_pairs() {
        let segment = Segment::new(vec![p(0.0, 0.0), p(0.0, 1.0), p(0.0, 2.0)]);
        let single_step = distance(&p(0.0, 0.0), &p(0.0, 1.0));
        assert!((segment_length(&segment) - 2.0 * single_step).abs() < 1.0);
    }

    #[test]
    fn short_segments_have_no_length() {
        assert_eq!(segment_length(&Segment::new(vec![])), 0.0);
        assert_eq!(segment_length(&Segment::new(vec![p(1.0, 1.0)])), 0.0);
    }

    /// Distance must not be summed across a segment boundary — a paused
    /// recording that resumes 40 km away has not traveled those 40 km.
    #[test]
    fn trace_length_does_not_bridge_segment_gaps() {
        let trace = Trace {
            metadata: Default::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![
                    Segment::new(vec![p(0.0, 0.0), p(0.0, 0.01)]),
                    Segment::new(vec![p(0.5, 0.5), p(0.5, 0.51)]),
                ],
            }],
        };
        let expected =
            distance(&p(0.0, 0.0), &p(0.0, 0.01)) + distance(&p(0.5, 0.5), &p(0.5, 0.51));
        assert!((trace_length(&trace) - expected).abs() < 1e-6);
    }
}
