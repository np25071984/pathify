use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{Metadata, Point};

/// A parsed file's worth of trace data.
///
/// The hierarchy mirrors GPX (the most expressive of the five supported
/// formats), so every adapter's read path is a widening conversion. Formats
/// with less structure — CSV, GeoJSON, a flat FIT record stream — collapse into
/// a single track with a single segment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub metadata: Metadata,
    pub tracks: Vec<Track>,
}

/// A named route or activity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub name: Option<String>,
    pub description: Option<String>,
    pub segments: Vec<Segment>,
}

/// A run of points assumed contiguous in time.
///
/// Segment boundaries are where a recording paused or lost signal, and they are
/// the unit that drift-cleaning and dedup operate within.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Segment {
    pub points: Vec<Point>,
}

/// A latitude/longitude bounding box, in WGS84 degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Bounds {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl Trace {
    pub fn new(metadata: Metadata, tracks: Vec<Track>) -> Self {
        Self { metadata, tracks }
    }

    /// Every point in the trace, in track then segment then recorded order.
    ///
    /// Double-ended so callers that need the last point — trimming the end of a
    /// journey, say — do not have to walk the whole trace to reach it.
    pub fn points(&self) -> impl DoubleEndedIterator<Item = &Point> {
        self.tracks
            .iter()
            .flat_map(|track| track.segments.iter())
            .flat_map(|segment| segment.points.iter())
    }

    /// Every segment in the trace, in track order.
    pub fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.tracks.iter().flat_map(|track| track.segments.iter())
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn segment_count(&self) -> usize {
        self.tracks.iter().map(|t| t.segments.len()).sum()
    }

    pub fn point_count(&self) -> usize {
        self.segments().map(|s| s.points.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.point_count() == 0
    }

    /// Earliest and latest timestamps across the trace, if any point has one.
    ///
    /// Computed by scanning rather than assuming points are sorted — sources
    /// merged from several devices are not guaranteed to arrive in time order.
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let mut times = self.points().filter_map(|p| p.time);
        let first = times.next()?;
        Some(times.fold((first, first), |(min, max), t| (min.min(t), max.max(t))))
    }

    /// Bounding box of every point, or `None` for an empty trace.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut points = self.points();
        let first = points.next()?;
        Some(points.fold(
            Bounds {
                min_lat: first.lat,
                min_lon: first.lon,
                max_lat: first.lat,
                max_lon: first.lon,
            },
            |b, p| Bounds {
                min_lat: b.min_lat.min(p.lat),
                min_lon: b.min_lon.min(p.lon),
                max_lat: b.max_lat.max(p.lat),
                max_lon: b.max_lon.max(p.lon),
            },
        ))
    }
}

impl Track {
    pub fn point_count(&self) -> usize {
        self.segments.iter().map(|s| s.points.len()).sum()
    }
}

impl Segment {
    pub fn new(points: Vec<Point>) -> Self {
        Self { points }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn point(lat: f64, lon: f64) -> Point {
        Point::new(lat, lon).unwrap()
    }

    fn sample() -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![
                Track {
                    name: Some("first".into()),
                    description: None,
                    segments: vec![
                        Segment::new(vec![point(1.0, 2.0), point(3.0, 4.0)]),
                        Segment::new(vec![point(-5.0, 6.0)]),
                    ],
                },
                Track {
                    name: None,
                    description: None,
                    segments: vec![Segment::new(vec![point(7.0, -8.0)])],
                },
            ],
        }
    }

    #[test]
    fn counts_walk_the_whole_hierarchy() {
        let trace = sample();
        assert_eq!(trace.track_count(), 2);
        assert_eq!(trace.segment_count(), 3);
        assert_eq!(trace.point_count(), 4);
        assert!(!trace.is_empty());
        assert!(Trace::default().is_empty());
    }

    #[test]
    fn bounds_cover_every_point() {
        let b = sample().bounds().unwrap();
        assert_eq!(b.min_lat, -5.0);
        assert_eq!(b.max_lat, 7.0);
        assert_eq!(b.min_lon, -8.0);
        assert_eq!(b.max_lon, 6.0);
        assert!(Trace::default().bounds().is_none());
    }

    #[test]
    fn time_range_ignores_point_order() {
        let later = Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap();
        let earlier = Utc.with_ymd_and_hms(2024, 5, 1, 11, 0, 0).unwrap();
        let trace = Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![Segment::new(vec![
                    point(0.0, 0.0).with_time(Some(later)),
                    point(0.0, 1.0).with_time(Some(earlier)),
                    point(0.0, 2.0),
                ])],
            }],
        };
        assert_eq!(trace.time_range(), Some((earlier, later)));
        assert_eq!(Trace::default().time_range(), None);
    }
}
