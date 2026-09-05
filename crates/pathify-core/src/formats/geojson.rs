//! GeoJSON adapter.
//!
//! GeoJSON has no native place for a per-point timestamp — the spec's positions
//! are `[lon, lat]` or `[lon, lat, elevation]` and stop there. Rather than drop
//! time on export, this adapter reads and writes `coordTimes`, the de-facto
//! convention (a property array of RFC 3339 timestamps running parallel to the
//! coordinates) used by Mapbox's tooling and most GPX-to-GeoJSON converters.
//!
//! A file without `coordTimes` still reads fine; its points simply have no time.

use std::io::Write;

use chrono::{DateTime, Utc};
use geojson::{Feature, FeatureCollection, GeoJson, Geometry, GeometryValue, JsonObject, Position};

use crate::error::{Error, Result};
use crate::model::{Metadata, Point, Segment, Trace, Track};

use super::Format;

/// Parse GeoJSON bytes into the trace model.
///
/// `LineString` becomes a one-segment track and `MultiLineString` a
/// multi-segment one, which is the mapping that survives a round trip through
/// the model. Standalone `Point` features are gathered into a single track so a
/// waypoint export is not silently empty.
pub fn read(bytes: &[u8]) -> Result<Trace> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error::Parse {
        format: "GeoJSON",
        message: e.to_string(),
    })?;
    let parsed: GeoJson = text.parse().map_err(|e: geojson::Error| Error::Parse {
        format: "GeoJSON",
        message: e.to_string(),
    })?;

    let mut tracks = Vec::new();
    let mut loose_points = Vec::new();

    match parsed {
        GeoJson::FeatureCollection(collection) => {
            for feature in collection.features {
                absorb_feature(feature, &mut tracks, &mut loose_points)?;
            }
        }
        GeoJson::Feature(feature) => absorb_feature(feature, &mut tracks, &mut loose_points)?,
        GeoJson::Geometry(geometry) => {
            absorb_geometry(&geometry, None, None, &mut tracks, &mut loose_points)?
        }
    }

    if !loose_points.is_empty() {
        tracks.push(Track {
            name: None,
            description: None,
            segments: vec![Segment::new(loose_points)],
        });
    }

    Ok(Trace {
        metadata: Metadata::default().with_source_format(Format::GeoJson),
        tracks,
    })
}

/// Serialize a trace as a GeoJSON `FeatureCollection`, one feature per track.
pub fn write(trace: &Trace, out: &mut dyn Write) -> Result<()> {
    let features: Vec<Feature> = trace
        .tracks
        .iter()
        .filter(|track| track.point_count() > 0)
        .map(track_to_feature)
        .collect();

    let collection = GeoJson::FeatureCollection(FeatureCollection::new(features));
    let text = collection.to_string_pretty().map_err(|e| Error::Write {
        format: "GeoJSON",
        message: e.to_string(),
    })?;
    out.write_all(text.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

fn absorb_feature(
    feature: Feature,
    tracks: &mut Vec<Track>,
    loose_points: &mut Vec<Point>,
) -> Result<()> {
    let name = feature
        .properties
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let times = feature.properties.as_ref().and_then(coord_times);

    match &feature.geometry {
        Some(geometry) => absorb_geometry(geometry, name, times, tracks, loose_points),
        None => Ok(()),
    }
}

fn absorb_geometry(
    geometry: &Geometry,
    name: Option<String>,
    times: Option<CoordTimes>,
    tracks: &mut Vec<Track>,
    loose_points: &mut Vec<Point>,
) -> Result<()> {
    match &geometry.value {
        GeometryValue::LineString { coordinates } => {
            let times = times.and_then(CoordTimes::into_flat);
            tracks.push(Track {
                name,
                description: None,
                segments: vec![Segment::new(positions_to_points(
                    coordinates,
                    times.as_deref(),
                )?)],
            });
        }
        GeometryValue::MultiLineString { coordinates } => {
            let nested = times.map(CoordTimes::into_nested);
            let mut segments = Vec::with_capacity(coordinates.len());
            for (i, line) in coordinates.iter().enumerate() {
                let line_times = nested.as_ref().and_then(|n| n.get(i)).map(Vec::as_slice);
                segments.push(Segment::new(positions_to_points(line, line_times)?));
            }
            tracks.push(Track {
                name,
                description: None,
                segments,
            });
        }
        GeometryValue::Point { coordinates } => {
            let times = times.and_then(CoordTimes::into_flat);
            loose_points.push(position_to_point(
                coordinates,
                times.as_deref().and_then(|t| t.first().copied().flatten()),
            )?);
        }
        GeometryValue::MultiPoint { coordinates } => {
            let times = times.and_then(CoordTimes::into_flat);
            loose_points.extend(positions_to_points(coordinates, times.as_deref())?);
        }
        GeometryValue::GeometryCollection { geometries } => {
            for inner in geometries {
                absorb_geometry(inner, name.clone(), None, tracks, loose_points)?;
            }
        }
        // Polygons and multipolygons are areas, not traces. Ignoring them is
        // deliberate: turning a boundary ring into a "route" would invent a
        // path nobody traveled.
        _ => {}
    }
    Ok(())
}

fn positions_to_points(
    positions: &[Position],
    times: Option<&[Option<DateTime<Utc>>]>,
) -> Result<Vec<Point>> {
    positions
        .iter()
        .enumerate()
        .map(|(i, position)| {
            let time = times.and_then(|t| t.get(i).copied().flatten());
            position_to_point(position, time)
        })
        .collect()
}

fn position_to_point(position: &Position, time: Option<DateTime<Utc>>) -> Result<Point> {
    let values = position.as_slice();
    if values.len() < 2 {
        return Err(Error::Parse {
            format: "GeoJSON",
            message: format!("position needs at least [lon, lat], got {values:?}"),
        });
    }
    // RFC 7946 orders positions longitude first.
    Ok(Point::new(values[1], values[0])?
        .with_elevation(values.get(2).copied())
        .with_time(time))
}

fn track_to_feature(track: &Track) -> Feature {
    let mut properties = JsonObject::new();
    if let Some(name) = &track.name {
        properties.insert("name".into(), name.clone().into());
    }

    let lines: Vec<Vec<Position>> = track
        .segments
        .iter()
        .map(|segment| segment.points.iter().map(point_to_position).collect())
        .collect();

    // Only emit coordTimes when there is at least one real timestamp to carry;
    // an array of nulls would be noise.
    if track
        .segments
        .iter()
        .flat_map(|s| &s.points)
        .any(|p| p.time.is_some())
    {
        let times: Vec<serde_json::Value> = track
            .segments
            .iter()
            .map(|segment| {
                let stamps: Vec<serde_json::Value> = segment
                    .points
                    .iter()
                    .map(|p| match p.time {
                        Some(t) => serde_json::Value::String(t.to_rfc3339()),
                        None => serde_json::Value::Null,
                    })
                    .collect();
                serde_json::Value::Array(stamps)
            })
            .collect();

        // Match the shape of the geometry: flat for a single segment, nested
        // for several, the same way readers expect to find it.
        let value = if times.len() == 1 {
            times.into_iter().next().expect("length checked")
        } else {
            serde_json::Value::Array(times)
        };
        properties.insert("coordTimes".into(), value);
    }

    let geometry = if lines.len() == 1 {
        Geometry::new(GeometryValue::LineString {
            coordinates: lines.into_iter().next().expect("length checked"),
        })
    } else {
        Geometry::new(GeometryValue::MultiLineString { coordinates: lines })
    };

    Feature {
        bbox: None,
        geometry: Some(geometry),
        id: None,
        properties: Some(properties),
        foreign_members: None,
    }
}

fn point_to_position(point: &Point) -> Position {
    match point.elevation {
        Some(elevation) => Position::from([point.lon, point.lat, elevation]),
        None => Position::from([point.lon, point.lat]),
    }
}

/// `coordTimes` appears either flat (one array, matching a LineString) or
/// nested (an array per line, matching a MultiLineString).
enum CoordTimes {
    Flat(Vec<Option<DateTime<Utc>>>),
    Nested(Vec<Vec<Option<DateTime<Utc>>>>),
}

impl CoordTimes {
    fn into_flat(self) -> Option<Vec<Option<DateTime<Utc>>>> {
        match self {
            CoordTimes::Flat(times) => Some(times),
            // A nested array against a flat geometry is malformed; the first
            // line is the best available reading.
            CoordTimes::Nested(nested) => nested.into_iter().next(),
        }
    }

    fn into_nested(self) -> Vec<Vec<Option<DateTime<Utc>>>> {
        match self {
            CoordTimes::Flat(times) => vec![times],
            CoordTimes::Nested(nested) => nested,
        }
    }
}

fn coord_times(properties: &JsonObject) -> Option<CoordTimes> {
    let value = properties
        .get("coordTimes")
        .or_else(|| properties.get("times"))?;
    let array = value.as_array()?;

    if array.first().is_some_and(|first| first.is_array()) {
        Some(CoordTimes::Nested(
            array
                .iter()
                .map(|line| {
                    line.as_array()
                        .map(|l| l.iter().map(parse_time).collect())
                        .unwrap_or_default()
                })
                .collect(),
        ))
    } else {
        Some(CoordTimes::Flat(array.iter().map(parse_time).collect()))
    }
}

fn parse_time(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"{
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "properties": {
          "name": "Outbound",
          "coordTimes": ["2024-05-01T15:00:00Z", "2024-05-01T15:00:30Z"]
        },
        "geometry": {
          "type": "LineString",
          "coordinates": [[-122.3321, 47.6062, 56.0], [-122.3330, 47.6070, 61.0]]
        }
      }]
    }"#;

    #[test]
    fn reads_a_linestring_with_elevation_and_times() {
        let trace = read(LINE.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.segment_count(), 1);
        assert_eq!(trace.point_count(), 2);
        assert_eq!(trace.tracks[0].name.as_deref(), Some("Outbound"));

        let first = trace.points().next().unwrap();
        assert!((first.lat - 47.6062).abs() < 1e-9);
        assert!((first.lon - -122.3321).abs() < 1e-9);
        assert_eq!(first.elevation, Some(56.0));
        assert_eq!(
            first.time.unwrap().to_rfc3339(),
            "2024-05-01T15:00:00+00:00"
        );
    }

    #[test]
    fn reads_a_multilinestring_as_multiple_segments() {
        let json = r#"{
          "type": "Feature",
          "properties": {},
          "geometry": {
            "type": "MultiLineString",
            "coordinates": [[[1.0, 2.0], [1.1, 2.1]], [[3.0, 4.0]]]
          }
        }"#;
        let trace = read(json.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.segment_count(), 2);
        assert_eq!(trace.point_count(), 3);
    }

    #[test]
    fn a_position_without_elevation_stays_absent_not_zero() {
        let json = r#"{"type":"LineString","coordinates":[[1.0,2.0],[1.1,2.1]]}"#;
        let trace = read(json.as_bytes()).unwrap();
        assert!(trace.points().all(|p| p.elevation.is_none()));
    }

    #[test]
    fn gathers_standalone_points() {
        let json = r#"{
          "type": "FeatureCollection",
          "features": [
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1.0,2.0]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[3.0,4.0]}}
          ]
        }"#;
        let trace = read(json.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.point_count(), 2);
    }

    /// A polygon is an area, not a path; turning it into a track would invent
    /// a route that was never traveled.
    #[test]
    fn ignores_polygons() {
        let json = r#"{
          "type":"Feature","properties":{},
          "geometry":{"type":"Polygon","coordinates":[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]}
        }"#;
        let trace = read(json.as_bytes()).unwrap();
        assert_eq!(trace.point_count(), 0);
    }

    #[test]
    fn round_trips_through_the_model() {
        let original = read(LINE.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        write(&original, &mut buffer).unwrap();
        let reparsed = read(&buffer).unwrap();

        assert_eq!(reparsed.track_count(), original.track_count());
        assert_eq!(reparsed.segment_count(), original.segment_count());
        assert_eq!(reparsed.tracks[0].name, original.tracks[0].name);
        for (a, b) in original.points().zip(reparsed.points()) {
            assert!((a.lat - b.lat).abs() < 1e-9);
            assert!((a.lon - b.lon).abs() < 1e-9);
            assert_eq!(a.elevation, b.elevation);
            assert_eq!(a.time, b.time, "coordTimes should survive the round trip");
        }
    }

    #[test]
    fn round_trips_multiple_segments_with_nested_times() {
        let mut original = read(LINE.as_bytes()).unwrap();
        let extra = Segment::new(vec![
            Point::new(1.0, 2.0).unwrap().with_time(Some(
                DateTime::parse_from_rfc3339("2024-05-01T16:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )),
        ]);
        original.tracks[0].segments.push(extra);

        let mut buffer = Vec::new();
        write(&original, &mut buffer).unwrap();
        let reparsed = read(&buffer).unwrap();

        assert_eq!(reparsed.segment_count(), 2);
        assert_eq!(reparsed.point_count(), 3);
        let times: Vec<_> = reparsed.points().map(|p| p.time).collect();
        assert!(
            times.iter().all(Option::is_some),
            "each segment keeps its times"
        );
    }

    #[test]
    fn omits_coord_times_when_no_point_has_one() {
        let json = r#"{"type":"LineString","coordinates":[[1.0,2.0]]}"#;
        let trace = read(json.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        write(&trace, &mut buffer).unwrap();
        let text = String::from_utf8(buffer).unwrap();
        assert!(!text.contains("coordTimes"), "{text}");
    }

    #[test]
    fn reports_a_parse_error_rather_than_panicking() {
        assert!(matches!(
            read(b"{not json").unwrap_err(),
            Error::Parse {
                format: "GeoJSON",
                ..
            }
        ));
    }
}
