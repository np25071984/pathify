//! CSV adapter.
//!
//! CSV is the one supported format that does not describe itself, so this
//! adapter fixes a convention rather than guessing per file.
//!
//! **Writing** emits a stable header:
//!
//! ```text
//! track,segment,lat,lon,ele,time
//! ```
//!
//! `track` and `segment` are zero-based indexes. They exist so the model's
//! hierarchy survives a round trip — without them, a paused recording would
//! come back as one unbroken segment and its gap would silently become
//! distance. Absent values are written as empty cells, never as `0`.
//!
//! **Reading** matches columns by header name, accepting the common aliases
//! other tools emit (`latitude`, `altitude`, `timestamp`, and so on). Only
//! latitude and longitude are required; a file without `track`/`segment`
//! columns reads as a single segment.

use std::collections::HashMap;
use std::io::Write;

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::error::{Error, Result};
use crate::model::{Metadata, Point, Segment, Trace, Track};

use super::Format;

/// Header written by [`write`], and the canonical spelling of each column.
pub const COLUMNS: [&str; 6] = ["track", "segment", "lat", "lon", "ele", "time"];

/// Header spellings accepted on read, mapped to the canonical column.
const ALIASES: &[(&str, &str)] = &[
    ("track", "track"),
    ("track_id", "track"),
    ("trackindex", "track"),
    ("segment", "segment"),
    ("segment_id", "segment"),
    ("seg", "segment"),
    ("lat", "lat"),
    ("latitude", "lat"),
    ("y", "lat"),
    ("lon", "lon"),
    ("lng", "lon"),
    ("long", "lon"),
    ("longitude", "lon"),
    ("x", "lon"),
    ("ele", "ele"),
    ("elevation", "ele"),
    ("altitude", "ele"),
    ("alt", "ele"),
    ("time", "time"),
    ("timestamp", "time"),
    ("datetime", "time"),
];

/// Parse CSV bytes into the trace model.
pub fn read(bytes: &[u8]) -> Result<Trace> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(bytes);

    let headers = reader.headers().map_err(parse_error)?.clone();
    let index = column_index(&headers)?;

    // Points are grouped by (track, segment) in first-seen order, so the file's
    // own ordering is preserved rather than sorted into index order.
    let mut order: Vec<(i64, i64)> = Vec::new();
    let mut groups: HashMap<(i64, i64), Vec<Point>> = HashMap::new();

    for (row, record) in reader.records().enumerate() {
        let record = record.map_err(parse_error)?;
        let line = row + 2; // header is line 1

        let lat = required_number(&record, index.lat, "lat", line)?;
        let lon = required_number(&record, index.lon, "lon", line)?;
        let point = Point::new(lat, lon)?
            .with_elevation(index.ele.and_then(|i| optional_number(&record, i)))
            .with_time(match index.time.and_then(|i| non_empty(&record, i)) {
                Some(text) => Some(parse_time(text, line)?),
                None => None,
            });

        let key = (
            index
                .track
                .and_then(|i| optional_number(&record, i))
                .unwrap_or(0.0) as i64,
            index
                .segment
                .and_then(|i| optional_number(&record, i))
                .unwrap_or(0.0) as i64,
        );
        if !groups.contains_key(&key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(point);
    }

    // Segments of the same track stay together, in first-seen order.
    let mut tracks: Vec<Track> = Vec::new();
    let mut track_positions: HashMap<i64, usize> = HashMap::new();
    for key in order {
        let points = groups.remove(&key).unwrap_or_default();
        let position = *track_positions.entry(key.0).or_insert_with(|| {
            tracks.push(Track::default());
            tracks.len() - 1
        });
        tracks[position].segments.push(Segment::new(points));
    }

    Ok(Trace {
        metadata: Metadata::default().with_source_format(Format::Csv),
        tracks,
    })
}

/// Serialize a trace as CSV with the header documented on this module.
pub fn write(trace: &Trace, out: &mut dyn Write) -> Result<()> {
    let mut writer = csv::Writer::from_writer(out);
    writer.write_record(COLUMNS).map_err(write_error)?;

    for (track_index, track) in trace.tracks.iter().enumerate() {
        for (segment_index, segment) in track.segments.iter().enumerate() {
            for point in &segment.points {
                writer
                    .write_record([
                        track_index.to_string(),
                        segment_index.to_string(),
                        format_coordinate(point.lat),
                        format_coordinate(point.lon),
                        point.elevation.map(|e| format!("{e}")).unwrap_or_default(),
                        point.time.map(|t| t.to_rfc3339()).unwrap_or_default(),
                    ])
                    .map_err(write_error)?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

#[derive(Debug, Default)]
struct ColumnIndex {
    track: Option<usize>,
    segment: Option<usize>,
    lat: usize,
    lon: usize,
    ele: Option<usize>,
    time: Option<usize>,
}

/// The canonical column a header names, or `None` for one to ignore.
///
/// `header` must already be normalized the way [`column_index`] does it:
/// trimmed, lowercased, with spaces and hyphens turned into underscores.
/// Shared with the Takeout reader, which recognizes the same coordinate
/// columns but has one more of its own to pick out.
pub(crate) fn canonical_column(header: &str) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == header)
        .map(|(_, canonical)| *canonical)
}

fn column_index(headers: &csv::StringRecord) -> Result<ColumnIndex> {
    let mut found: HashMap<&str, usize> = HashMap::new();
    for (i, header) in headers.iter().enumerate() {
        let normalized = header.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        if let Some(canonical) = canonical_column(&normalized) {
            // First matching column wins, so a file with both `lat` and
            // `latitude` does not flip-flop between them.
            found.entry(canonical).or_insert(i);
        }
    }

    let (Some(&lat), Some(&lon)) = (found.get("lat"), found.get("lon")) else {
        return Err(Error::Parse {
            format: "CSV",
            message: format!(
                "no latitude/longitude columns found in header [{}]; expected columns named {}",
                headers.iter().collect::<Vec<_>>().join(", "),
                COLUMNS.join(", ")
            ),
        });
    };

    Ok(ColumnIndex {
        track: found.get("track").copied(),
        segment: found.get("segment").copied(),
        lat,
        lon,
        ele: found.get("ele").copied(),
        time: found.get("time").copied(),
    })
}

fn non_empty(record: &csv::StringRecord, index: usize) -> Option<&str> {
    record.get(index).map(str::trim).filter(|s| !s.is_empty())
}

fn optional_number(record: &csv::StringRecord, index: usize) -> Option<f64> {
    non_empty(record, index).and_then(|s| s.parse().ok())
}

fn required_number(
    record: &csv::StringRecord,
    index: usize,
    column: &str,
    line: usize,
) -> Result<f64> {
    let text = non_empty(record, index).ok_or_else(|| Error::Parse {
        format: "CSV",
        message: format!("line {line}: missing a value for `{column}`"),
    })?;
    text.parse().map_err(|_| Error::Parse {
        format: "CSV",
        message: format!("line {line}: `{text}` in column `{column}` is not a number"),
    })
}

/// Accept RFC 3339 first, then the space-separated spelling that spreadsheets
/// and database exports produce, which is otherwise the most common reason a
/// perfectly good file fails to import.
fn parse_time(text: &str, line: usize) -> Result<DateTime<Utc>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Ok(parsed.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(naive.and_utc());
        }
    }
    Err(Error::Parse {
        format: "CSV",
        message: format!("line {line}: `{text}` is not a recognized timestamp (expected RFC 3339)"),
    })
}

/// Coordinates are written at full precision; `{}` on an f64 would render
/// 47.6062 as `47.6062` but risks scientific notation on small magnitudes.
fn format_coordinate(value: f64) -> String {
    let text = format!("{value:.9}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_error(e: csv::Error) -> Error {
    Error::Parse {
        format: "CSV",
        message: e.to_string(),
    }
}

fn write_error(e: csv::Error) -> Error {
    Error::Write {
        format: "CSV",
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_canonical_header() {
        let csv = "track,segment,lat,lon,ele,time\n\
                   0,0,47.6062,-122.3321,56,2024-05-01T15:00:00Z\n\
                   0,0,47.6070,-122.3330,61,2024-05-01T15:00:30Z\n";
        let trace = read(csv.as_bytes()).unwrap();
        assert_eq!(trace.point_count(), 2);
        assert_eq!(trace.segment_count(), 1);

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
    fn accepts_common_header_aliases() {
        let csv = "Latitude,Longitude,Altitude,Timestamp\n\
                   47.6062,-122.3321,56,2024-05-01T15:00:00Z\n";
        let trace = read(csv.as_bytes()).unwrap();
        assert_eq!(trace.point_count(), 1);
        let point = trace.points().next().unwrap();
        assert_eq!(point.elevation, Some(56.0));
        assert!(point.time.is_some());
    }

    #[test]
    fn a_file_without_track_columns_is_one_segment() {
        let csv = "lat,lon\n1.0,2.0\n1.1,2.1\n1.2,2.2\n";
        let trace = read(csv.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        assert_eq!(trace.segment_count(), 1);
        assert_eq!(trace.point_count(), 3);
    }

    #[test]
    fn track_and_segment_columns_rebuild_the_hierarchy() {
        let csv = "track,segment,lat,lon\n\
                   0,0,1.0,2.0\n\
                   0,1,1.1,2.1\n\
                   1,0,1.2,2.2\n";
        let trace = read(csv.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 2);
        assert_eq!(trace.segment_count(), 3);
        assert_eq!(trace.tracks[0].segments.len(), 2);
        assert_eq!(trace.tracks[1].segments.len(), 1);
    }

    #[test]
    fn empty_cells_stay_absent_rather_than_becoming_zero() {
        let csv = "lat,lon,ele,time\n1.0,2.0,,\n";
        let trace = read(csv.as_bytes()).unwrap();
        let point = trace.points().next().unwrap();
        assert_eq!(point.elevation, None);
        assert_eq!(point.time, None);
    }

    #[test]
    fn accepts_a_space_separated_timestamp() {
        let csv = "lat,lon,time\n1.0,2.0,2024-05-01 15:00:00\n";
        let trace = read(csv.as_bytes()).unwrap();
        assert_eq!(
            trace.points().next().unwrap().time.unwrap().to_rfc3339(),
            "2024-05-01T15:00:00+00:00"
        );
    }

    #[test]
    fn a_header_without_coordinates_explains_what_was_expected() {
        let err = read(b"name,notes\nfoo,bar\n").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("latitude/longitude"), "{message}");
        assert!(message.contains("lat"), "{message}");
    }

    #[test]
    fn a_bad_number_names_its_line_and_column() {
        let err = read(b"lat,lon\n1.0,2.0\nnorth,2.1\n").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("line 3"), "{message}");
        assert!(message.contains("lat"), "{message}");
    }

    #[test]
    fn a_bad_timestamp_is_an_error_not_a_silent_drop() {
        let err = read(b"lat,lon,time\n1.0,2.0,yesterday\n").unwrap_err();
        assert!(err.to_string().contains("timestamp"), "{err}");
    }

    #[test]
    fn writes_the_documented_header_and_leaves_absent_values_empty() {
        let csv = "lat,lon\n1.5,2.5\n";
        let trace = read(csv.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        write(&trace, &mut buffer).unwrap();
        let text = String::from_utf8(buffer).unwrap();

        assert!(
            text.starts_with("track,segment,lat,lon,ele,time\n"),
            "{text}"
        );
        assert!(text.contains("0,0,1.5,2.5,,"), "{text}");
    }

    #[test]
    fn round_trips_structure_and_values() {
        let csv = "track,segment,lat,lon,ele,time\n\
                   0,0,47.6062,-122.3321,56,2024-05-01T15:00:00Z\n\
                   0,1,47.607,-122.333,,\n\
                   1,0,47.608,-122.334,61.5,2024-05-01T15:01:00Z\n";
        let original = read(csv.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        write(&original, &mut buffer).unwrap();
        let reparsed = read(&buffer).unwrap();

        assert_eq!(reparsed.track_count(), original.track_count());
        assert_eq!(reparsed.segment_count(), original.segment_count());
        for (a, b) in original.points().zip(reparsed.points()) {
            assert!((a.lat - b.lat).abs() < 1e-9);
            assert!((a.lon - b.lon).abs() < 1e-9);
            assert_eq!(a.elevation, b.elevation);
            assert_eq!(a.time, b.time);
        }
    }

    #[test]
    fn writes_coordinates_without_scientific_notation() {
        assert_eq!(format_coordinate(47.6062), "47.6062");
        assert_eq!(format_coordinate(-122.3321), "-122.3321");
        assert_eq!(format_coordinate(0.0), "0");
        assert_eq!(format_coordinate(0.000000123), "0.000000123");
    }
}
