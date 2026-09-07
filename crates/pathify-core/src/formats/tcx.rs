//! TCX (Training Center XML) adapter.
//!
//! TCX organizes a workout as `Activities > Activity > Lap > Track >
//! Trackpoint` rather than GPX's flatter `Track > TrackSegment > Trackpoint`.
//! The mapping onto the model treats each `Activity` as a [`Track`] and each
//! `Track` element — GPS-loss boundaries within a `Lap`, and there can be more
//! than one — as a [`Segment`], which is the same reason GPX's `<trkseg>`
//! becomes one: it is where the source itself already recorded a break.
//!
//! Reading tolerates the shapes that real exporters produce: a `Trackpoint`
//! with no `<Position>` (common in indoor activities recorded on a trainer)
//! is dropped rather than invented as `(0, 0)`, and extension elements like
//! `TPX/Speed` are matched by local name only, so it does not matter which
//! namespace prefix a given exporter happens to use for them.
//!
//! Writing has nowhere to put a `Sport`, so every activity is written as
//! `Sport="Other"` — the model has no equivalent field, and TCX requires the
//! attribute to be present.

use std::io::Write;

use chrono::{DateTime, Utc};
use xml::reader::EventReader;
use xml::reader::XmlEvent as ReadEvent;
use xml::writer::{EmitterConfig, XmlEvent as WriteEvent};

use crate::error::{Error, Result};
use crate::model::{Extras, Metadata, Point, Segment, Trace, Track};

use super::Format;

const TCX_NAMESPACE: &str = "http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2";
const TPX_NAMESPACE: &str = "http://www.garmin.com/xmlschemas/ActivityExtension/v2";

/// Parse TCX bytes into the trace model.
pub fn read(bytes: &[u8]) -> Result<Trace> {
    let reader = EventReader::new(bytes);

    let mut tracks = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();

    let mut activity_description: Option<String> = None;
    let mut activity_segments: Vec<Segment> = Vec::new();
    let mut segment_points: Vec<Point> = Vec::new();

    let mut point_lat: Option<f64> = None;
    let mut point_lon: Option<f64> = None;
    let mut point_ele: Option<f64> = None;
    let mut point_time: Option<DateTime<Utc>> = None;
    let mut point_hr: Option<u16> = None;
    let mut point_cadence: Option<u16> = None;
    let mut point_speed: Option<f64> = None;

    let mut first_activity_id: Option<String> = None;
    let mut author_name: Option<String> = None;
    let mut creator_name: Option<String> = None;

    for event in reader {
        let event = event.map_err(|e| Error::Parse {
            format: "TCX",
            message: e.to_string(),
        })?;

        match event {
            ReadEvent::StartElement { name, .. } => {
                stack.push(name.local_name);
                text.clear();
                if ends_with(&stack, &["Activity"]) {
                    activity_description = None;
                    activity_segments = Vec::new();
                }
                if ends_with(&stack, &["Trackpoint"]) {
                    point_lat = None;
                    point_lon = None;
                    point_ele = None;
                    point_time = None;
                    point_hr = None;
                    point_cadence = None;
                    point_speed = None;
                }
            }
            ReadEvent::Characters(chunk) => text.push_str(&chunk),
            ReadEvent::EndElement { .. } => {
                let name = stack.pop().unwrap_or_default();
                let value = text.trim();

                match name.as_str() {
                    "LatitudeDegrees" if ends_with(&stack, &["Position"]) => {
                        point_lat = value.parse().ok();
                    }
                    "LongitudeDegrees" if ends_with(&stack, &["Position"]) => {
                        point_lon = value.parse().ok();
                    }
                    "AltitudeMeters" if ends_with(&stack, &["Trackpoint"]) => {
                        point_ele = value.parse().ok();
                    }
                    "Time" if ends_with(&stack, &["Trackpoint"]) => {
                        point_time = parse_time(value);
                    }
                    "Value" if ends_with(&stack, &["HeartRateBpm"]) => {
                        point_hr = value.parse().ok();
                    }
                    "Cadence" if ends_with(&stack, &["Trackpoint"]) => {
                        point_cadence = value.parse().ok();
                    }
                    // Matched by local name alone: exporters disagree on the
                    // namespace prefix for the TPX extension block.
                    "Speed" if stack.last().map(String::as_str) == Some("TPX") => {
                        point_speed = value.parse().ok();
                    }
                    "Notes" if ends_with(&stack, &["Activity"]) => {
                        activity_description = Some(value.to_string());
                    }
                    "Id" if ends_with(&stack, &["Activity"]) => {
                        first_activity_id.get_or_insert_with(|| value.to_string());
                    }
                    "Name" if ends_with(&stack, &["Author"]) => {
                        author_name = Some(value.to_string());
                    }
                    "Name" if ends_with(&stack, &["Creator"]) => {
                        creator_name = Some(value.to_string());
                    }
                    "Trackpoint" => {
                        // No <Position> — an indoor trackpoint, say — cannot
                        // become a model Point, which requires coordinates.
                        // Dropping it beats inventing (0, 0).
                        if let (Some(lat), Some(lon)) = (point_lat, point_lon)
                            && let Ok(point) = Point::new(lat, lon)
                        {
                            let mut point = point.with_elevation(point_ele).with_time(point_time);
                            point.extras = Extras {
                                heart_rate: point_hr,
                                cadence: point_cadence,
                                speed: point_speed,
                                ..Extras::default()
                            };
                            segment_points.push(point);
                        }
                    }
                    "Track" => {
                        activity_segments.push(Segment::new(std::mem::take(&mut segment_points)));
                    }
                    "Activity" => {
                        tracks.push(Track {
                            name: None,
                            description: activity_description.take(),
                            segments: std::mem::take(&mut activity_segments),
                        });
                    }
                    _ => {}
                }
                text.clear();
            }
            _ => {}
        }
    }

    let metadata = Metadata {
        name: None,
        description: None,
        source_format: Some(Format::Tcx),
        source_device: creator_name.or(author_name),
        created: first_activity_id.as_deref().and_then(parse_time),
    };

    Ok(Trace { metadata, tracks })
}

/// Serialize a trace as TCX, one `Activity` per track and one `Track` element
/// per segment.
pub fn write(trace: &Trace, out: &mut dyn Write) -> Result<()> {
    let mut writer = EmitterConfig::new().perform_indent(true).create_writer(out);

    let write_error = |e: xml::writer::Error| Error::Write {
        format: "TCX",
        message: e.to_string(),
    };

    writer
        .write(WriteEvent::start_element("TrainingCenterDatabase").default_ns(TCX_NAMESPACE))
        .map_err(write_error)?;
    writer
        .write(WriteEvent::start_element("Activities"))
        .map_err(write_error)?;

    for track in &trace.tracks {
        write_activity(&mut writer, track).map_err(write_error)?;
    }

    // Activities, then TrainingCenterDatabase.
    writer
        .write(WriteEvent::end_element())
        .map_err(write_error)?;
    writer
        .write(WriteEvent::end_element())
        .map_err(write_error)?;
    Ok(())
}

fn write_activity(
    writer: &mut xml::writer::EventWriter<&mut dyn Write>,
    track: &Track,
) -> std::result::Result<(), xml::writer::Error> {
    // TCX has no field for a track's own name or description; `Notes` is the
    // closest fit, and `Sport` and `Id` are required even when the model has
    // nothing to put in them.
    let id = track
        .segments
        .iter()
        .flat_map(|s| &s.points)
        .find_map(|p| p.time)
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());

    writer.write(WriteEvent::start_element("Activity").attr("Sport", "Other"))?;
    writer.write(WriteEvent::start_element("Id"))?;
    writer.write(WriteEvent::characters(&id))?;
    writer.write(WriteEvent::end_element())?; // Id

    if let Some(description) = &track.description {
        writer.write(WriteEvent::start_element("Notes"))?;
        writer.write(WriteEvent::characters(description))?;
        writer.write(WriteEvent::end_element())?; // Notes
    }

    for segment in &track.segments {
        let start = segment
            .points
            .first()
            .and_then(|p| p.time)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| id.clone());

        writer.write(WriteEvent::start_element("Lap").attr("StartTime", start.as_str()))?;
        writer.write(WriteEvent::start_element("Track"))?;
        for point in &segment.points {
            write_trackpoint(writer, point)?;
        }
        writer.write(WriteEvent::end_element())?; // Track
        writer.write(WriteEvent::end_element())?; // Lap
    }

    writer.write(WriteEvent::end_element()) // Activity
}

fn write_trackpoint(
    writer: &mut xml::writer::EventWriter<&mut dyn Write>,
    point: &Point,
) -> std::result::Result<(), xml::writer::Error> {
    writer.write(WriteEvent::start_element("Trackpoint"))?;

    if let Some(time) = point.time {
        writer.write(WriteEvent::start_element("Time"))?;
        writer.write(WriteEvent::characters(&time.to_rfc3339()))?;
        writer.write(WriteEvent::end_element())?;
    }

    writer.write(WriteEvent::start_element("Position"))?;
    writer.write(WriteEvent::start_element("LatitudeDegrees"))?;
    writer.write(WriteEvent::characters(&point.lat.to_string()))?;
    writer.write(WriteEvent::end_element())?;
    writer.write(WriteEvent::start_element("LongitudeDegrees"))?;
    writer.write(WriteEvent::characters(&point.lon.to_string()))?;
    writer.write(WriteEvent::end_element())?;
    writer.write(WriteEvent::end_element())?; // Position

    if let Some(elevation) = point.elevation {
        writer.write(WriteEvent::start_element("AltitudeMeters"))?;
        writer.write(WriteEvent::characters(&elevation.to_string()))?;
        writer.write(WriteEvent::end_element())?;
    }

    if let Some(hr) = point.extras.heart_rate {
        writer.write(WriteEvent::start_element("HeartRateBpm"))?;
        writer.write(WriteEvent::start_element("Value"))?;
        writer.write(WriteEvent::characters(&hr.to_string()))?;
        writer.write(WriteEvent::end_element())?; // Value
        writer.write(WriteEvent::end_element())?; // HeartRateBpm
    }

    if let Some(cadence) = point.extras.cadence {
        writer.write(WriteEvent::start_element("Cadence"))?;
        writer.write(WriteEvent::characters(&cadence.to_string()))?;
        writer.write(WriteEvent::end_element())?;
    }

    if let Some(speed) = point.extras.speed {
        writer.write(WriteEvent::start_element("Extensions"))?;
        writer.write(WriteEvent::start_element("TPX").default_ns(TPX_NAMESPACE))?;
        writer.write(WriteEvent::start_element("Speed"))?;
        writer.write(WriteEvent::characters(&speed.to_string()))?;
        writer.write(WriteEvent::end_element())?; // Speed
        writer.write(WriteEvent::end_element())?; // TPX
        writer.write(WriteEvent::end_element())?; // Extensions
    }

    writer.write(WriteEvent::end_element()) // Trackpoint
}

/// Whether `stack`'s last elements are exactly `suffix`, in order — i.e.
/// whether the element just closed is a direct child of `suffix`'s last name.
fn ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack[stack.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(a, b)| a == b)
}

/// TCX timestamps are always RFC 3339 (the schema requires `xsd:dateTime`).
fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<TrainingCenterDatabase xmlns="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2"
                         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Activities>
    <Activity Sport="Biking">
      <Id>2024-05-01T15:00:00Z</Id>
      <Notes>Morning ride</Notes>
      <Lap StartTime="2024-05-01T15:00:00Z">
        <Track>
          <Trackpoint>
            <Time>2024-05-01T15:00:00Z</Time>
            <Position>
              <LatitudeDegrees>47.6062</LatitudeDegrees>
              <LongitudeDegrees>-122.3321</LongitudeDegrees>
            </Position>
            <AltitudeMeters>56.0</AltitudeMeters>
            <HeartRateBpm><Value>120</Value></HeartRateBpm>
            <Cadence>85</Cadence>
            <Extensions>
              <ns3:TPX xmlns:ns3="http://www.garmin.com/xmlschemas/ActivityExtension/v2">
                <ns3:Speed>5.5</ns3:Speed>
              </ns3:TPX>
            </Extensions>
          </Trackpoint>
          <Trackpoint>
            <Time>2024-05-01T15:00:30Z</Time>
            <Position>
              <LatitudeDegrees>47.6070</LatitudeDegrees>
              <LongitudeDegrees>-122.3330</LongitudeDegrees>
            </Position>
          </Trackpoint>
        </Track>
        <Track>
          <Trackpoint>
            <Time>2024-05-01T15:05:00Z</Time>
            <Position>
              <LatitudeDegrees>47.6100</LatitudeDegrees>
              <LongitudeDegrees>-122.3400</LongitudeDegrees>
            </Position>
          </Trackpoint>
        </Track>
      </Lap>
    </Activity>
  </Activities>
  <Author xsi:type="Application_t">
    <Name>TestDevice</Name>
  </Author>
</TrainingCenterDatabase>
"#;

    #[test]
    fn reads_activities_laps_and_trackpoints() {
        let trace = read(SAMPLE.as_bytes()).unwrap();
        assert_eq!(trace.track_count(), 1);
        // Two <Track> elements under the one <Lap>, from a GPS gap mid-ride.
        assert_eq!(trace.segment_count(), 2);
        assert_eq!(trace.point_count(), 3);
        assert_eq!(trace.tracks[0].description.as_deref(), Some("Morning ride"));
        assert_eq!(trace.metadata.source_format, Some(Format::Tcx));
        assert_eq!(trace.metadata.source_device.as_deref(), Some("TestDevice"));
        assert_eq!(
            trace.metadata.created.unwrap().to_rfc3339(),
            "2024-05-01T15:00:00+00:00"
        );
    }

    #[test]
    fn reads_coordinates_elevation_time_and_extras() {
        let trace = read(SAMPLE.as_bytes()).unwrap();
        let first = trace.points().next().unwrap();
        assert!((first.lat - 47.6062).abs() < 1e-9);
        assert!((first.lon - -122.3321).abs() < 1e-9);
        assert_eq!(first.elevation, Some(56.0));
        assert_eq!(
            first.time.unwrap().to_rfc3339(),
            "2024-05-01T15:00:00+00:00"
        );
        assert_eq!(first.extras.heart_rate, Some(120));
        assert_eq!(first.extras.cadence, Some(85));
        assert_eq!(first.extras.speed, Some(5.5));

        // A trackpoint with none of the optional fields keeps them absent.
        let second = trace.points().nth(1).unwrap();
        assert_eq!(second.elevation, None);
        assert_eq!(second.extras.heart_rate, None);
    }

    #[test]
    fn a_trackpoint_without_position_is_dropped_not_invented() {
        let indoor = r#"<?xml version="1.0"?>
<TrainingCenterDatabase xmlns="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2">
  <Activities>
    <Activity Sport="Other">
      <Id>2024-05-01T15:00:00Z</Id>
      <Lap StartTime="2024-05-01T15:00:00Z">
        <Track>
          <Trackpoint>
            <Time>2024-05-01T15:00:00Z</Time>
            <HeartRateBpm><Value>140</Value></HeartRateBpm>
          </Trackpoint>
          <Trackpoint>
            <Time>2024-05-01T15:00:01Z</Time>
            <Position>
              <LatitudeDegrees>1.0</LatitudeDegrees>
              <LongitudeDegrees>2.0</LongitudeDegrees>
            </Position>
          </Trackpoint>
        </Track>
      </Lap>
    </Activity>
  </Activities>
</TrainingCenterDatabase>"#;
        let trace = read(indoor.as_bytes()).unwrap();
        assert_eq!(
            trace.point_count(),
            1,
            "the GPS-less point should be dropped"
        );
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
            assert_eq!(a.extras.heart_rate, b.extras.heart_rate);
            assert_eq!(a.extras.cadence, b.extras.cadence);
            assert_eq!(a.extras.speed, b.extras.speed);
        }
    }

    /// TCX has nowhere to put a track's `name` — only its `description`
    /// survives, via `<Notes>`. Documented on the module and pinned here so a
    /// future change does not invent a field to smuggle it into.
    #[test]
    fn a_tracks_name_does_not_survive_the_round_trip_but_its_description_does() {
        let mut original = read(SAMPLE.as_bytes()).unwrap();
        original.tracks[0].name = Some("Outbound".to_string());

        let mut buffer = Vec::new();
        write(&original, &mut buffer).unwrap();
        let reparsed = read(&buffer).unwrap();

        assert_eq!(reparsed.tracks[0].name, None);
        assert_eq!(
            reparsed.tracks[0].description,
            original.tracks[0].description
        );
    }

    #[test]
    fn reports_a_parse_error_rather_than_panicking() {
        let err = read(b"<TrainingCenterDatabase><Activities>").unwrap_err();
        assert!(matches!(err, Error::Parse { format: "TCX", .. }));
    }

    #[test]
    fn a_file_with_no_activities_reads_as_empty_rather_than_an_error() {
        let empty = br#"<?xml version="1.0"?><TrainingCenterDatabase xmlns="x"><Activities/></TrainingCenterDatabase>"#;
        let trace = read(empty).unwrap();
        assert!(trace.is_empty());
    }
}
