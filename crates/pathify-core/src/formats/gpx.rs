//! GPX adapter.
//!
//! GPX is the format whose shape the internal model was designed around, so
//! this adapter is close to a straight structural mapping. The one piece of
//! real impedance matching is time: the `gpx` crate speaks `time`, the model
//! speaks `chrono`.

use std::io::Write;

use chrono::{DateTime, Utc};
use gpx::{Gpx, GpxVersion, Time, Track as GpxTrack, TrackSegment, Waypoint};
use time::OffsetDateTime;

use crate::error::{Error, Result};
use crate::model::{Extras, Metadata, Point, Segment, Trace, Track};

use super::Format;

/// Parse GPX bytes into the trace model.
pub fn read(bytes: &[u8]) -> Result<Trace> {
    let parsed: Gpx = gpx::read(bytes).map_err(|e| Error::Parse {
        format: "GPX",
        message: e.to_string(),
    })?;

    let mut tracks: Vec<Track> = parsed
        .tracks
        .iter()
        .map(|track| Track {
            name: track.name.clone(),
            description: track.description.clone(),
            segments: track
                .segments
                .iter()
                .map(|segment| Segment::new(segment.points.iter().map(waypoint_to_point).collect()))
                .collect(),
        })
        .collect();

    // Loose waypoints and routes would otherwise be dropped silently. A route
    // is an ordered list of points, so it maps onto a track cleanly; scattered
    // waypoints only become one when there are no tracks to speak of.
    for route in &parsed.routes {
        tracks.push(Track {
            name: route.name.clone(),
            description: route.description.clone(),
            segments: vec![Segment::new(
                route.points.iter().map(waypoint_to_point).collect(),
            )],
        });
    }
    if tracks.is_empty() && !parsed.waypoints.is_empty() {
        tracks.push(Track {
            name: None,
            description: None,
            segments: vec![Segment::new(
                parsed.waypoints.iter().map(waypoint_to_point).collect(),
            )],
        });
    }

    let metadata = Metadata {
        name: parsed.metadata.as_ref().and_then(|m| m.name.clone()),
        description: parsed.metadata.as_ref().and_then(|m| m.description.clone()),
        source_format: Some(Format::Gpx),
        source_device: parsed.creator.clone(),
        created: parsed
            .metadata
            .as_ref()
            .and_then(|m| m.time)
            .and_then(time_to_chrono),
    };

    Ok(Trace { metadata, tracks })
}

/// Serialize a trace as GPX 1.1.
pub fn write(trace: &Trace, out: &mut dyn Write) -> Result<()> {
    let mut document = Gpx {
        version: GpxVersion::Gpx11,
        creator: Some(
            trace
                .metadata
                .source_device
                .clone()
                .unwrap_or_else(|| format!("pathify {}", env!("CARGO_PKG_VERSION"))),
        ),
        ..Default::default()
    };

    if trace.metadata.name.is_some()
        || trace.metadata.description.is_some()
        || trace.metadata.created.is_some()
    {
        document.metadata = Some(gpx::Metadata {
            name: trace.metadata.name.clone(),
            description: trace.metadata.description.clone(),
            time: trace.metadata.created.and_then(chrono_to_time),
            ..Default::default()
        });
    }

    document.tracks = trace
        .tracks
        .iter()
        .map(|track| GpxTrack {
            name: track.name.clone(),
            description: track.description.clone(),
            segments: track
                .segments
                .iter()
                .map(|segment| TrackSegment {
                    points: segment.points.iter().map(point_to_waypoint).collect(),
                })
                .collect(),
            ..Default::default()
        })
        .collect();

    gpx::write(&document, out).map_err(|e| Error::Write {
        format: "GPX",
        message: e.to_string(),
    })
}

fn waypoint_to_point(waypoint: &Waypoint) -> Point {
    let geo_point = waypoint.point();
    // Coordinates from a parsed file can be anything; `Point::new` normalizes
    // longitude and rejects nonsense, and a point we can't represent is
    // clamped rather than dropped so indexes into the source stay aligned.
    let mut point = Point::new(geo_point.y(), geo_point.x())
        .unwrap_or_else(|_| Point::new(0.0, 0.0).expect("origin is a valid coordinate"));

    point.elevation = waypoint.elevation;
    point.time = waypoint.time.and_then(time_to_chrono);
    point.extras = Extras {
        speed: waypoint.speed,
        hdop: waypoint.hdop,
        satellites: waypoint.sat.and_then(|s| u16::try_from(s).ok()),
        ..Extras::default()
    };
    point
}

fn point_to_waypoint(point: &Point) -> Waypoint {
    let mut waypoint = Waypoint::new(geo::Point::new(point.lon, point.lat));
    waypoint.elevation = point.elevation;
    waypoint.time = point.time.and_then(chrono_to_time);
    waypoint.speed = point.extras.speed;
    waypoint.hdop = point.extras.hdop;
    waypoint.sat = point.extras.satellites.map(u64::from);
    waypoint
}

fn time_to_chrono(t: Time) -> Option<DateTime<Utc>> {
    let offset: OffsetDateTime = t.into();
    DateTime::from_timestamp(offset.unix_timestamp(), offset.nanosecond())
}

fn chrono_to_time(t: DateTime<Utc>) -> Option<Time> {
    OffsetDateTime::from_unix_timestamp(t.timestamp())
        .ok()
        .map(Time::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="TestDevice" xmlns="http://www.topografix.com/GPX/1/1">
  <metadata><name>Morning ride</name></metadata>
  <trk>
    <name>Leg one</name>
    <trkseg>
      <trkpt lat="47.6062" lon="-122.3321"><ele>56.0</ele><time>2024-05-01T15:00:00Z</time></trkpt>
      <trkpt lat="47.6070" lon="-122.3330"><ele>61.0</ele><time>2024-05-01T15:00:30Z</time></trkpt>
    </trkseg>
    <trkseg>
      <trkpt lat="47.6100" lon="-122.3400"></trkpt>
    </trkseg>
  </trk>
</gpx>
"#;

    #[test]
    fn reads_tracks_segments_and_points() {
        let trace = read(SAMPLE.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.segment_count(), 2);
        assert_eq!(trace.point_count(), 3);
        assert_eq!(trace.tracks[0].name.as_deref(), Some("Leg one"));
        assert_eq!(trace.metadata.name.as_deref(), Some("Morning ride"));
        assert_eq!(trace.metadata.source_device.as_deref(), Some("TestDevice"));
        assert_eq!(trace.metadata.source_format, Some(Format::Gpx));
    }

    #[test]
    fn reads_coordinates_elevation_and_time() {
        let trace = read(SAMPLE.as_bytes()).unwrap();
        let first = trace.points().next().unwrap();
        assert!((first.lat - 47.6062).abs() < 1e-9);
        assert!((first.lon - -122.3321).abs() < 1e-9);
        assert_eq!(first.elevation, Some(56.0));
        assert_eq!(
            first.time.unwrap().to_rfc3339(),
            "2024-05-01T15:00:00+00:00"
        );

        // A point without <ele>/<time> keeps them absent rather than zeroed.
        let last = trace.points().last().unwrap();
        assert_eq!(last.elevation, None);
        assert_eq!(last.time, None);
    }

    #[test]
    fn write_then_read_preserves_the_model() {
        let original = read(SAMPLE.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        write(&original, &mut buffer).unwrap();
        let reparsed = read(&buffer).unwrap();

        assert_eq!(reparsed.track_count(), original.track_count());
        assert_eq!(reparsed.segment_count(), original.segment_count());
        assert_eq!(reparsed.point_count(), original.point_count());
        for (a, b) in original.points().zip(reparsed.points()) {
            assert!((a.lat - b.lat).abs() < 1e-9);
            assert!((a.lon - b.lon).abs() < 1e-9);
            assert_eq!(a.elevation, b.elevation);
            assert_eq!(a.time, b.time);
        }
        assert_eq!(reparsed.metadata.name, original.metadata.name);
    }

    #[test]
    fn reports_a_parse_error_rather_than_panicking() {
        let err = read(b"<gpx><trk>").unwrap_err();
        assert!(matches!(err, Error::Parse { format: "GPX", .. }));
    }

    #[test]
    fn keeps_routes_and_loose_waypoints() {
        let with_route = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="t" xmlns="http://www.topografix.com/GPX/1/1">
  <wpt lat="1.0" lon="2.0"></wpt>
  <rte><name>Planned</name><rtept lat="3.0" lon="4.0"></rtept></rte>
</gpx>"#;
        let trace = read(with_route.as_bytes()).unwrap();
        // The route becomes a track; the waypoint is not also folded in.
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.tracks[0].name.as_deref(), Some("Planned"));
        assert_eq!(trace.point_count(), 1);
    }

    #[test]
    fn waypoint_only_file_still_yields_points() {
        let only_waypoints = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="t" xmlns="http://www.topografix.com/GPX/1/1">
  <wpt lat="1.0" lon="2.0"><ele>5</ele></wpt>
  <wpt lat="1.5" lon="2.5"></wpt>
</gpx>"#;
        let trace = read(only_waypoints.as_bytes()).unwrap();
        assert_eq!(trace.point_count(), 2);
    }
}
