//! Deciding whether two fixes from different sources are the same moment.
//!
//! This is the matching rule behind `merge`'s reconciliation mode. Getting it
//! wrong is quiet: match too eagerly and real points disappear, match too
//! reluctantly and the output carries visible duplicate jitter that doubles the
//! reported distance. Both failures look like a working merge.

use chrono::Duration;

use crate::model::{Extras, Point};

use super::distance::distance;
use super::drift::{MAX_PLAUSIBLE_SPEED_MPS, coincidence_radius_m};

/// Narrowest matching window worth using.
///
/// Below about a second, clock skew between two independent devices dominates
/// the sampling interval, and a tighter window would reject genuine matches.
pub const MIN_MATCH_WINDOW: Duration = Duration::seconds(1);

/// Widest matching window, however coarse the sampling.
///
/// Past half a minute apart, "the same moment" stops being a meaningful claim
/// about a moving subject.
pub const MAX_MATCH_WINDOW: Duration = Duration::seconds(30);

/// Median gap between consecutive timestamped points, i.e. the sampling rate.
///
/// The median rather than the mean because recordings pause: a single lunch
/// break would drag an average interval far past anything the device does.
pub fn median_interval<'a>(points: impl IntoIterator<Item = &'a Point>) -> Option<Duration> {
    let times: Vec<_> = points.into_iter().filter_map(|p| p.time).collect();
    if times.len() < 2 {
        return None;
    }

    let mut gaps: Vec<i64> = times
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).num_milliseconds().abs())
        .filter(|ms| *ms > 0)
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_unstable();
    Some(Duration::milliseconds(gaps[gaps.len() / 2]))
}

/// The time window within which two points may be considered the same moment.
///
/// Half the coarser sampling interval, which is exactly where "nearest sample"
/// tips over to the neighbouring one: a 30-second recording paired against a
/// 1 Hz one gets a 15-second window, so each coarse sample claims the fine
/// sample nearest it and no further.
pub fn match_window(a: Option<Duration>, b: Option<Duration>) -> Duration {
    let coarsest = match (a, b) {
        (Some(a), Some(b)) => a.max(b),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => return MIN_MATCH_WINDOW,
    };
    (coarsest / 2).clamp(MIN_MATCH_WINDOW, MAX_MATCH_WINDOW)
}

/// Whether `candidate` can be the same physical moment as `reference`.
///
/// Returns the absolute time difference on a match, so a caller comparing
/// several candidates can keep the closest. Points without timestamps never
/// match: with no time axis there is nothing to reconcile against.
pub fn same_moment(
    reference: &Point,
    candidate: &Point,
    window: Duration,
    radius_m: Option<f64>,
) -> Option<Duration> {
    let (a, b) = (reference.time?, candidate.time?);
    let apart = if a > b { a - b } else { b - a };
    if apart > window {
        return None;
    }

    let limit = radius_m.unwrap_or_else(|| coincidence_radius_m(apart, MAX_PLAUSIBLE_SPEED_MPS));
    (distance(reference, candidate) <= limit).then_some(apart)
}

/// Collapse two fixes of the same moment into one.
///
/// Position, elevation, and timestamp come from `primary` — picking a winner
/// beats averaging, which would invent a location neither device recorded. The
/// optional extras union instead: a value present in either source is kept,
/// because a watch's heart rate and a phone's better fix are complementary, and
/// dropping the one the primary happens to lack loses data for no reason.
pub fn merge_points(primary: &Point, other: &Point) -> Point {
    Point {
        lat: primary.lat,
        lon: primary.lon,
        elevation: primary.elevation.or(other.elevation),
        time: primary.time.or(other.time),
        extras: Extras {
            heart_rate: primary.extras.heart_rate.or(other.extras.heart_rate),
            cadence: primary.extras.cadence.or(other.extras.cadence),
            speed: primary.extras.speed.or(other.extras.speed),
            hdop: primary.extras.hdop.or(other.extras.hdop),
            satellites: primary.extras.satellites.or(other.extras.satellites),
            temperature: primary.extras.temperature.or(other.extras.temperature),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn at(lat: f64, lon: f64, seconds: i64) -> Point {
        Point::new(lat, lon).unwrap().with_time(Some(
            Utc.with_ymd_and_hms(2024, 5, 1, 15, 0, 0).unwrap() + Duration::seconds(seconds),
        ))
    }

    #[test]
    fn median_interval_reads_the_sampling_rate() {
        let points = vec![at(0.0, 0.0, 0), at(0.0, 0.0, 5), at(0.0, 0.0, 10)];
        assert_eq!(median_interval(&points), Some(Duration::seconds(5)));
    }

    /// A pause in the middle must not drag the estimate: the mean of these gaps
    /// is over 100 s, the median is 1 s, and 1 s is the device's actual rate.
    #[test]
    fn median_interval_ignores_a_long_pause() {
        let points = vec![
            at(0.0, 0.0, 0),
            at(0.0, 0.0, 1),
            at(0.0, 0.0, 2),
            at(0.0, 0.0, 602),
            at(0.0, 0.0, 603),
        ];
        assert_eq!(median_interval(&points), Some(Duration::seconds(1)));
    }

    #[test]
    fn median_interval_needs_two_timestamped_points() {
        assert_eq!(median_interval(&[]), None);
        assert_eq!(median_interval(&[at(0.0, 0.0, 0)]), None);
        let timeless = vec![Point::new(0.0, 0.0).unwrap(), Point::new(1.0, 1.0).unwrap()];
        assert_eq!(median_interval(&timeless), None);
    }

    #[test]
    fn window_is_half_the_coarser_interval() {
        let window = match_window(Some(Duration::seconds(30)), Some(Duration::seconds(1)));
        assert_eq!(window, Duration::seconds(15));
    }

    #[test]
    fn window_is_clamped_at_both_ends() {
        // Very fine sampling still gets a usable floor.
        assert_eq!(
            match_window(Some(Duration::milliseconds(200)), None),
            MIN_MATCH_WINDOW
        );
        // Very coarse sampling does not stretch the claim indefinitely.
        assert_eq!(
            match_window(Some(Duration::seconds(600)), None),
            MAX_MATCH_WINDOW
        );
        assert_eq!(match_window(None, None), MIN_MATCH_WINDOW);
    }

    #[test]
    fn matches_a_near_simultaneous_nearby_fix() {
        let reference = at(47.6062, -122.3321, 0);
        let candidate = at(47.60621, -122.33211, 2); // ~1 m away, 2 s later
        assert_eq!(
            same_moment(&reference, &candidate, Duration::seconds(15), None),
            Some(Duration::seconds(2))
        );
    }

    #[test]
    fn rejects_a_fix_outside_the_window() {
        let reference = at(47.6062, -122.3321, 0);
        let candidate = at(47.6062, -122.3321, 30); // same place, too late
        assert_eq!(
            same_moment(&reference, &candidate, Duration::seconds(15), None),
            None
        );
    }

    #[test]
    fn rejects_a_simultaneous_fix_that_is_too_far_away() {
        let reference = at(47.6062, -122.3321, 0);
        let far = at(47.7062, -122.3321, 0); // ~11 km away at the same instant
        assert_eq!(
            same_moment(&reference, &far, Duration::seconds(15), None),
            None
        );
    }

    #[test]
    fn an_explicit_radius_overrides_the_speed_derived_one() {
        let reference = at(47.6062, -122.3321, 0);
        let candidate = at(47.6072, -122.3321, 2); // ~111 m away

        // The default bound is permissive enough to accept it.
        assert!(same_moment(&reference, &candidate, Duration::seconds(15), None).is_some());
        // A tight explicit radius does not.
        assert!(same_moment(&reference, &candidate, Duration::seconds(15), Some(10.0)).is_none());
    }

    #[test]
    fn points_without_timestamps_never_match() {
        let timeless = Point::new(47.6062, -122.3321).unwrap();
        let timed = at(47.6062, -122.3321, 0);
        assert!(same_moment(&timeless, &timed, Duration::seconds(15), None).is_none());
        assert!(same_moment(&timed, &timeless, Duration::seconds(15), None).is_none());
    }

    #[test]
    fn merging_keeps_the_primary_position_and_unions_extras() {
        let mut primary = at(47.6062, -122.3321, 0);
        primary.extras.hdop = Some(1.2);

        let mut other = at(47.6070, -122.3330, 1);
        other.elevation = Some(56.0);
        other.extras.heart_rate = Some(142);
        other.extras.hdop = Some(9.9);

        let merged = merge_points(&primary, &other);
        assert_eq!(merged.lat, primary.lat);
        assert_eq!(merged.lon, primary.lon);
        assert_eq!(merged.time, primary.time);
        // Present in only one source, so it survives.
        assert_eq!(merged.elevation, Some(56.0));
        assert_eq!(merged.extras.heart_rate, Some(142));
        // Present in both, so the primary wins.
        assert_eq!(merged.extras.hdop, Some(1.2));
    }
}
