//! Backend A: Google Health / Fitbit exports.
//!
//! Two file families carry everything this needs, and neither is any use
//! without the other:
//!
//! - `Physical Activity_GoogleData/gps_location_YYYY-MM-DD.csv` — the
//!   coordinates, one file per calendar day, at roughly 1 Hz.
//! - `Global Export Data/exercise-*.json` — the activity logs, which are what
//!   says that a particular seventy minutes of that day was a bike ride.
//!
//! So the work here is a join. A day file is not an activity: it runs from
//! midnight to midnight and can hold three unrelated outings, and it
//! interleaves every device that was recording, which for most people is two.
//! Reading one as-is gives a doubled, zigzagging track that wanders between
//! errands.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::formats::csv::canonical_column;
use crate::merge::split_on_gaps;
use crate::model::{Metadata, Point, Trace, Track};

use super::archive::{Archive, Entry};

/// How the exercise logs spell `startTime`: `MM/DD/YY hh:mm:ss`, with no
/// timezone marker at all. See [`detect_offset`] for what is done about that.
const START_TIME_FORMAT: &str = "%m/%d/%y %H:%M:%S";

/// Whether a file name is one of the exercise-log files.
///
/// Matched on the file name rather than the folder, because the folder is a
/// Takeout product name that has been renamed before and may be again.
pub fn is_exercise_log(file: &str) -> bool {
    file.strip_prefix("exercise-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// The calendar day a `gps_location_*.csv` covers, if that is what this is.
pub fn gps_day(file: &str) -> Option<NaiveDate> {
    let date = file
        .strip_prefix("gps_location_")
        .and_then(|rest| rest.strip_suffix(".csv"))?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

pub fn is_gps_day(file: &str) -> bool {
    gps_day(file).is_some()
}

/// One exercise log: an activity the user (or the tracker) recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityLog {
    /// Stable identifier, kept so a track stays traceable to its log after
    /// the trace has been merged with others.
    pub log_id: i64,
    /// Fitbit's own name for the activity — `Walk`, `Outdoor Bike`. Title
    /// case, with spaces, and nothing like Timeline's `IN_VEHICLE` enum.
    pub name: String,
    pub type_id: Option<i64>,
    /// The start as the export writes it: a wall clock with no offset.
    pub start: NaiveDateTime,
    pub duration: Duration,
    /// Whether the tracker says a GPS trace should exist for this log.
    pub has_gps: bool,
}

impl ActivityLog {
    /// The window this activity covers in UTC, given how the export's wall
    /// clock relates to it.
    pub fn window(&self, offset: Duration) -> (DateTime<Utc>, DateTime<Utc>) {
        let start = Utc.from_utc_datetime(&(self.start - offset));
        (start, start + self.duration)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLog {
    log_id: i64,
    activity_name: String,
    activity_type_id: Option<i64>,
    start_time: String,
    /// Milliseconds of actual activity, which is what the GPS trace covers;
    /// `duration` also counts time the recording was paused.
    active_duration: Option<i64>,
    duration: Option<i64>,
    #[serde(default)]
    has_gps: bool,
    // `tcxLink` is deliberately not read. It is a `fitbit.com` URL, and
    // Pathify has no network code and is not getting any — see the
    // no-network rule in CONTRIBUTING.md. Everything below is reconstructed
    // from what is already inside the archive.
}

/// Parse one `exercise-*.json` file.
pub fn read_logs(bytes: &[u8]) -> Result<Vec<ActivityLog>> {
    let raw: Vec<RawLog> = serde_json::from_slice(bytes).map_err(|error| Error::Parse {
        format: "Takeout exercise log",
        message: error.to_string(),
    })?;

    raw.into_iter()
        .map(|log| {
            let start = NaiveDateTime::parse_from_str(&log.start_time, START_TIME_FORMAT).map_err(
                |_| Error::Parse {
                    format: "Takeout exercise log",
                    message: format!(
                        "log {}: startTime `{}` is not in MM/DD/YY hh:mm:ss form",
                        log.log_id, log.start_time
                    ),
                },
            )?;
            Ok(ActivityLog {
                log_id: log.log_id,
                name: log.activity_name,
                type_id: log.activity_type_id,
                start,
                duration: Duration::milliseconds(
                    log.active_duration.or(log.duration).unwrap_or(0).max(0),
                ),
                has_gps: log.has_gps,
            })
        })
        .collect()
}

/// One activity type and how many of its logs can actually produce a track.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityType {
    pub name: String,
    pub with_gps: usize,
    pub without_gps: usize,
}

impl ActivityType {
    pub fn total(&self) -> usize {
        self.with_gps + self.without_gps
    }
}

/// Group logs by activity name.
///
/// Types with no GPS at all are still listed, with their count, rather than
/// hidden: a user who exported a year of swims needs to see that the swims
/// were found and have no coordinates, not that they vanished.
pub fn activity_types(logs: &[ActivityLog]) -> Vec<ActivityType> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for log in logs {
        let entry = counts.entry(log.name.as_str()).or_insert_with(|| {
            order.push(log.name.clone());
            (0, 0)
        });
        if log.has_gps {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }

    let mut types: Vec<ActivityType> = order
        .into_iter()
        .map(|name| {
            let (with_gps, without_gps) = counts[name.as_str()];
            ActivityType {
                name,
                with_gps,
                without_gps,
            }
        })
        .collect();
    // The ones something can be done with come first; the rest are context.
    types.sort_by(|a, b| {
        b.with_gps
            .cmp(&a.with_gps)
            .then(b.total().cmp(&a.total()))
            .then(a.name.cmp(&b.name))
    });
    types
}

/// Fold an activity name for matching, so `outdoor bike`, `Outdoor Bike` and
/// `outdoor_bike` are all the same request.
pub fn fold(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Requested type names that no log in the archive answers to.
///
/// Returned rather than ignored: emitting an empty trace because a name was
/// misspelled looks exactly like an archive that had nothing in it.
pub fn unmatched<'a>(types: &[ActivityType], wanted: &'a [String]) -> Vec<&'a str> {
    let known: Vec<String> = types.iter().map(|t| fold(&t.name)).collect();
    wanted
        .iter()
        .filter(|name| !known.contains(&fold(name)))
        .map(String::as_str)
        .collect()
}

/// A single row of a `gps_location_*.csv`, with the device that recorded it.
#[derive(Debug, Clone, PartialEq)]
struct Fix {
    point: Point,
    /// The `data source` column, e.g. `Google Fitbit Air`.
    source: String,
}

/// Parse one day file, sorted by time.
///
/// Pathify's general CSV reader already understands these columns — it maps
/// `latitude`, `longitude`, `altitude` and `timestamp` onto the trace model —
/// but it drops `data source`, and that column is the whole difference between
/// one journey and two devices' worth of it. So the column names come from the
/// shared alias table and only the row handling is local.
fn read_day(bytes: &[u8], day: NaiveDate) -> Result<Vec<Fix>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(bytes);

    let headers = reader
        .headers()
        .map_err(|error| day_error(day, error.to_string()))?
        .clone();

    let mut lat = None;
    let mut lon = None;
    let mut ele = None;
    let mut time = None;
    let mut source = None;
    for (index, header) in headers.iter().enumerate() {
        let normalized = header.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        if normalized == "data_source" || normalized == "source" {
            source.get_or_insert(index);
            continue;
        }
        let slot = match canonical_column(&normalized) {
            Some("lat") => &mut lat,
            Some("lon") => &mut lon,
            Some("ele") => &mut ele,
            Some("time") => &mut time,
            _ => continue,
        };
        slot.get_or_insert(index);
    }

    let (Some(lat), Some(lon), Some(time)) = (lat, lon, time) else {
        return Err(day_error(
            day,
            format!(
                "expected timestamp, latitude and longitude columns, found [{}]",
                headers.iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    };

    let mut fixes = Vec::new();
    for (row, record) in reader.records().enumerate() {
        let record = record.map_err(|error| day_error(day, error.to_string()))?;
        let line = row + 2; // the header is line 1

        let cell = |index: usize| record.get(index).map(str::trim).filter(|s| !s.is_empty());
        let number = |index: usize, column: &str| -> Result<f64> {
            cell(index)
                .ok_or_else(|| day_error(day, format!("line {line}: no value for `{column}`")))?
                .parse()
                .map_err(|_| day_error(day, format!("line {line}: `{column}` is not a number")))
        };

        let stamp = cell(time)
            .ok_or_else(|| day_error(day, format!("line {line}: no value for `timestamp`")))?;
        let recorded = DateTime::parse_from_rfc3339(stamp)
            .map_err(|_| {
                day_error(
                    day,
                    format!("line {line}: `{stamp}` is not an RFC 3339 time"),
                )
            })?
            .with_timezone(&Utc);

        let point = Point::new(number(lat, "latitude")?, number(lon, "longitude")?)?
            .with_elevation(ele.and_then(cell).and_then(|s| s.parse().ok()))
            .with_time(Some(recorded));

        fixes.push(Fix {
            point,
            source: source.and_then(cell).unwrap_or("unknown").to_string(),
        });
    }

    // Sorted once here so every window that lands in this day can be found by
    // bisection instead of another full scan.
    fixes.sort_by_key(|fix| fix.point.time);
    Ok(fixes)
}

fn day_error(day: NaiveDate, message: String) -> Error {
    Error::Parse {
        format: "Takeout GPS day file",
        message: format!("gps_location_{day}.csv: {message}"),
    }
}

/// The exercise logs and GPS day files an archive holds.
///
/// Built once, because the menu, `--list` and the extraction all need it and
/// the exercise files are the one thing that has to be parsed in full. They
/// are a few hundred kilobytes across three files — the 1.9 GB of an archive
/// is sleep, heart rate and everything else, none of which is ever read.
pub struct Catalog {
    pub logs: Vec<ActivityLog>,
    days: HashMap<NaiveDate, Entry>,
}

impl Catalog {
    /// Parse the exercise logs and index the day files.
    pub fn of(archive: &mut Archive) -> Result<Self> {
        let mut logs = Vec::new();
        for entry in archive.find(|entry| is_exercise_log(entry.file_name())) {
            let bytes = archive.read(&entry)?;
            logs.extend(read_logs(&bytes)?);
        }
        logs.sort_by_key(|log| (log.start, log.log_id));

        let days = archive
            .find(|entry| is_gps_day(entry.file_name()))
            .into_iter()
            .filter_map(|entry| gps_day(entry.file_name()).map(|day| (day, entry)))
            .collect();

        Ok(Self { logs, days })
    }

    pub fn types(&self) -> Vec<ActivityType> {
        activity_types(&self.logs)
    }

    pub fn logs_with_gps(&self) -> usize {
        self.logs.iter().filter(|log| log.has_gps).count()
    }

    pub fn day_count(&self) -> usize {
        self.days.len()
    }

    /// Logs of the requested types that claim a GPS trace, in start order.
    ///
    /// An empty request means every type, which is what `takeout` uses when a
    /// caller has already made the choice some other way.
    pub fn select(&self, types: &[String]) -> Vec<&ActivityLog> {
        let wanted: Vec<String> = types.iter().map(|t| fold(t)).collect();
        self.logs
            .iter()
            .filter(|log| log.has_gps)
            .filter(|log| wanted.is_empty() || wanted.contains(&fold(&log.name)))
            .collect()
    }
}

/// What `extract` should pull out.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Activity names to keep. Empty means every type.
    pub types: Vec<String>,
    /// Keep only this recording device where several logged the same journey.
    pub source: Option<String>,
    /// Gap that starts a new segment.
    pub segment_gap: Duration,
    /// Force the wall-clock-to-UTC offset instead of detecting it.
    pub offset: Option<Duration>,
}

/// What the extraction found, and what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub logs: usize,
    pub logs_with_gps: usize,
    pub selected: usize,
    pub tracks: usize,
    pub points: usize,
    /// Seconds the export's wall clock is ahead of UTC.
    pub offset_s: i64,
    /// Whether that offset was worked out from the data or just assumed.
    pub offset_detected: bool,
    /// Selected logs with no day file for the date they fall on.
    pub missing_days: Vec<(i64, NaiveDate)>,
    /// Selected logs whose day file held nothing inside their window.
    pub empty_windows: Vec<i64>,
    /// Per log, the device kept and the ones passed over.
    pub source_choices: Vec<(i64, String, Vec<String>)>,
}

/// One activity on its own, with the trace it produced.
///
/// [`extract`] welds every selected activity into one trace, which is what a
/// person piping it into `view` wants. A program importing each outing as its
/// own record wants the opposite, and carving the combined trace back up would
/// mean guessing at boundaries this module already knows exactly. So the seam
/// is here, beside the join, rather than downstream of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    /// The exercise log this came out of, unique within an archive.
    pub log_id: i64,
    /// The activity type as the export names it — `Walk`, `Outdoor Bike`.
    pub name: String,
    /// When the activity started, in UTC.
    pub start: DateTime<Utc>,
    /// The recording device that was kept for it.
    pub source: String,
    /// The trace, holding this activity's one track.
    pub trace: Trace,
}

/// Pull the selected activities out of a Health export as one trace.
pub fn extract(
    archive: &mut Archive,
    catalog: &Catalog,
    options: &Options,
) -> Result<(Trace, Report)> {
    let (activities, report, sources_seen) = gather(archive, catalog, options)?;

    let tracks: Vec<Track> = activities
        .into_iter()
        .flat_map(|activity| activity.trace.tracks)
        .collect();
    let created = tracks
        .first()
        .and_then(|track| track.segments.first())
        .and_then(|segment| segment.points.first())
        .and_then(|point| point.time);

    Ok((
        Trace::new(
            Metadata {
                name: Some("Google Takeout".to_string()),
                description: Some("Activities from a Google Health export".to_string()),
                source_device: (sources_seen.len() == 1).then(|| sources_seen[0].clone()),
                created,
                ..Metadata::default()
            },
            tracks,
        ),
        report,
    ))
}

/// The same extraction, one trace per activity instead of one for all of them.
///
/// Same selection, same slicing, same report — only the packaging differs.
pub fn extract_each(
    archive: &mut Archive,
    catalog: &Catalog,
    options: &Options,
) -> Result<(Vec<Activity>, Report)> {
    let (activities, report, _) = gather(archive, catalog, options)?;
    Ok((activities, report))
}

/// The join itself: each selected log sliced out of the day files it falls in.
///
/// The third return is every device seen across the whole selection, which
/// only [`extract`] has a use for — one activity names its own in
/// [`Activity::source`].
fn gather(
    archive: &mut Archive,
    catalog: &Catalog,
    options: &Options,
) -> Result<(Vec<Activity>, Report, Vec<String>)> {
    let selected = catalog.select(&options.types);
    let mut report = Report {
        logs: catalog.logs.len(),
        logs_with_gps: catalog.logs_with_gps(),
        selected: selected.len(),
        ..Report::default()
    };

    let mut days = DayCache::default();
    let offset = match options.offset {
        Some(offset) => offset,
        None => {
            let detected = detect_offset(archive, catalog, &selected, &mut days)?;
            report.offset_detected = detected.is_some();
            detected.unwrap_or_else(Duration::zero)
        }
    };
    report.offset_s = offset.num_seconds();

    let wanted_source = options.source.as_deref().map(str::to_ascii_lowercase);
    let mut sources_seen: Vec<String> = Vec::new();
    let mut activities = Vec::new();

    for log in &selected {
        let (start, end) = log.window(offset);
        // Only the days the window touches are decompressed, and the cache
        // keeps at most the two of them: an all-day file is megabytes, and a
        // year of walks would otherwise be held in memory at once.
        days.retain_from(start.date_naive());

        // Copied out of the cache rather than borrowed from it, so the next
        // day can be decompressed into the same cache while these are held.
        let mut fixes: Vec<Fix> = Vec::new();
        let mut missing = false;
        for day in days_touched(start, end) {
            match days.get(archive, catalog, day)? {
                Some(day_fixes) => fixes.extend_from_slice(window_of(day_fixes, start, end)),
                None => {
                    // Reported even when the other day of a window that
                    // straddles midnight did produce points: half a ride
                    // looks exactly like a whole short one.
                    missing = true;
                    report.missing_days.push((log.log_id, day));
                }
            }
        }

        if fixes.is_empty() {
            if !missing {
                report.empty_windows.push(log.log_id);
            }
            continue;
        }

        let (kept, passed_over) = choose_source(&fixes, wanted_source.as_deref());
        for source in std::iter::once(&kept).chain(passed_over.iter()) {
            if !sources_seen.contains(source) {
                sources_seen.push(source.clone());
            }
        }
        if !passed_over.is_empty() {
            report
                .source_choices
                .push((log.log_id, kept.clone(), passed_over));
        }

        let mut points: Vec<Point> = fixes
            .iter()
            .filter(|fix| fix.source == kept)
            .map(|fix| fix.point.clone())
            .collect();
        if points.is_empty() {
            continue;
        }
        points.sort_by_key(|point| point.time);

        report.points += points.len();
        let name = format!("{} {}", log.name, start.date_naive());
        let description = format!("Fitbit log {}", log.log_id);
        let track = Track {
            name: Some(name.clone()),
            description: Some(description.clone()),
            segments: split_on_gaps(points, options.segment_gap),
        };
        let created = track
            .segments
            .first()
            .and_then(|segment| segment.points.first())
            .and_then(|point| point.time);

        activities.push(Activity {
            log_id: log.log_id,
            name: log.name.clone(),
            start,
            source: kept.clone(),
            // The one-activity trace names itself after the activity, where
            // the combined one can only say which export it came from.
            trace: Trace::new(
                Metadata {
                    name: Some(name),
                    description: Some(description),
                    source_device: Some(kept),
                    created,
                    ..Metadata::default()
                },
                vec![track],
            ),
        });
    }

    // A `--source` that names a device this archive never recorded on is a
    // typo, and quietly emitting nothing would look like an empty export.
    if let Some(wanted) = &wanted_source
        && !sources_seen.is_empty()
        && !sources_seen
            .iter()
            .any(|seen| seen.to_ascii_lowercase() == *wanted)
    {
        return Err(Error::Takeout {
            message: format!(
                "no recordings from `{}`; this archive has {}",
                options.source.as_deref().unwrap_or_default(),
                sources_seen.join(", ")
            ),
        });
    }

    report.tracks = activities.len();
    Ok((activities, report, sources_seen))
}

/// The calendar days a UTC window falls across.
///
/// Usually one. An evening ride that runs past midnight UTC is in two files,
/// and reading only the first would cut the track off at the date line.
fn days_touched(start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<NaiveDate> {
    let (first, last) = (start.date_naive(), end.date_naive());
    let mut days = vec![first];
    let mut day = first;
    while day < last {
        day = day.succ_opt().unwrap_or(day);
        days.push(day);
        if days.len() > 366 {
            break; // a window this long is nonsense; stop rather than spin
        }
    }
    days
}

/// The fixes of a sorted day file that fall in `[start, end)`.
fn window_of(fixes: &[Fix], start: DateTime<Utc>, end: DateTime<Utc>) -> &[Fix] {
    let from = fixes.partition_point(|fix| fix.point.time.is_some_and(|t| t < start));
    let to = fixes.partition_point(|fix| fix.point.time.is_some_and(|t| t < end));
    &fixes[from..to.max(from)]
}

/// Decide which device's recording to keep for one activity.
///
/// Never a concatenation: the two are the same journey, and stringing them
/// together produces a track that zigzags between them and reports roughly
/// twice the distance.
fn choose_source(fixes: &[Fix], wanted: Option<&str>) -> (String, Vec<String>) {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for fix in fixes {
        match counts.iter_mut().find(|(name, _)| *name == fix.source) {
            Some((_, count)) => *count += 1,
            None => counts.push((fix.source.clone(), 1)),
        }
    }

    let chosen = wanted
        .and_then(|wanted| {
            counts
                .iter()
                .position(|(name, _)| name.to_ascii_lowercase() == wanted)
        })
        // Otherwise the one that saw the most of the journey, which is the
        // best available proxy for the recording that dropped out least.
        .or_else(|| {
            counts
                .iter()
                .enumerate()
                .max_by_key(|(_, (_, count))| *count)
                .map(|(index, _)| index)
        })
        .unwrap_or(0);

    let kept = counts[chosen].0.clone();
    let passed_over = counts
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != chosen)
        .map(|(_, (name, _))| name.clone())
        .collect();
    (kept, passed_over)
}

/// Day files read so far, keyed by date, with absent days remembered as such.
#[derive(Default)]
struct DayCache {
    days: HashMap<NaiveDate, Option<Vec<Fix>>>,
}

impl DayCache {
    fn get(
        &mut self,
        archive: &mut Archive,
        catalog: &Catalog,
        day: NaiveDate,
    ) -> Result<Option<&Vec<Fix>>> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.days.entry(day) {
            let parsed = match catalog.days.get(&day) {
                Some(entry) => {
                    let bytes = archive.read(entry)?;
                    Some(read_day(&bytes, day)?)
                }
                None => None,
            };
            slot.insert(parsed);
        }
        Ok(self.days[&day].as_ref())
    }

    /// Forget every day before `day`. Activities are processed in start
    /// order, so nothing earlier will be asked for again.
    fn retain_from(&mut self, day: NaiveDate) {
        self.days.retain(|held, _| *held >= day);
    }
}

/// How many sample activities the offset is worked out from.
const OFFSET_SAMPLES: usize = 8;

/// Offsets tried, in fifteen-minute steps across the inhabited range.
///
/// Ordered by size so that a tie — which is the normal case for a short
/// activity, since a nearby offset also lands on points — resolves to the
/// smallest shift, and to UTC before anything else.
fn candidate_offsets() -> Vec<Duration> {
    let mut minutes: Vec<i64> = (-12 * 4..=14 * 4).map(|step| step * 15).collect();
    minutes.sort_by_key(|m| (m.abs(), *m));
    minutes.into_iter().map(Duration::minutes).collect()
}

/// Work out how the exercise logs' wall clock relates to UTC.
///
/// `startTime` carries no offset, and Fitbit has historically written local
/// time there. In the archive this was written against it happens to be UTC,
/// but assuming that would mean silently emitting empty tracks for everyone it
/// is not true for. So the offset is measured: the one that puts the most
/// sample windows over actual recorded points wins.
///
/// Returns `None` when no offset lands on anything, which means the join has
/// failed for some other reason and the caller should say so rather than
/// present an assumption as a finding.
///
/// This is evidence, not proof, and it wants a few activities to work from: a
/// single ten-minute window can sit over an unrelated outing at some other
/// shift and score better there. [`Options::offset`] overrides it outright.
fn detect_offset(
    archive: &mut Archive,
    catalog: &Catalog,
    selected: &[&ActivityLog],
    days: &mut DayCache,
) -> Result<Option<Duration>> {
    let samples = sample(selected, OFFSET_SAMPLES);
    if samples.is_empty() {
        return Ok(None);
    }

    // The day a window falls in depends on the offset being tested, so the
    // neighbouring days are read too — that covers every candidate.
    let mut times: Vec<DateTime<Utc>> = Vec::new();
    for log in &samples {
        let middle = log.start.date();
        for day in [middle.pred_opt(), Some(middle), middle.succ_opt()]
            .into_iter()
            .flatten()
        {
            if let Some(fixes) = days.get(archive, catalog, day)? {
                times.extend(fixes.iter().filter_map(|fix| fix.point.time));
            }
        }
    }
    times.sort_unstable();

    // Scored on how many windows land on points first, and on how much of
    // each window is covered second. The second half matters: a small wrong
    // shift can clip the edge of an unrelated outing earlier in the day and
    // score a hit, but only the right one covers a window end to end.
    let mut best: Option<(usize, usize, Duration)> = None;
    for offset in candidate_offsets() {
        let mut windows = 0;
        let mut covered = 0;
        for log in &samples {
            let (start, end) = log.window(offset);
            let from = times.partition_point(|t| *t < start);
            let to = times.partition_point(|t| *t < end);
            if to > from {
                windows += 1;
                covered += to - from;
            }
        }
        if windows > 0 && (windows, covered) > best.map_or((0, 0), |(w, c, _)| (w, c)) {
            best = Some((windows, covered, offset));
        }
    }

    Ok(best.map(|(_, _, offset)| offset))
}

/// Up to `count` logs spread evenly across the selection.
///
/// Spread rather than the first few, because an archive can span a move
/// between timezones and the opening weeks are not representative of it.
fn sample<'a>(logs: &[&'a ActivityLog], count: usize) -> Vec<&'a ActivityLog> {
    if logs.len() <= count {
        return logs.to_vec();
    }
    (0..count).map(|i| logs[i * logs.len() / count]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(id: i64, name: &str, start: &str, minutes: i64, has_gps: bool) -> ActivityLog {
        ActivityLog {
            log_id: id,
            name: name.to_string(),
            type_id: None,
            start: NaiveDateTime::parse_from_str(start, START_TIME_FORMAT).unwrap(),
            duration: Duration::minutes(minutes),
            has_gps,
        }
    }

    #[test]
    fn recognizes_the_two_file_families_by_name() {
        assert!(is_exercise_log("exercise-100.json"));
        assert!(!is_exercise_log("exercise.json"));
        assert!(!is_exercise_log("UserExercises README.txt"));
        assert_eq!(
            gps_day("gps_location_2026-07-11.csv"),
            NaiveDate::from_ymd_opt(2026, 7, 11)
        );
        assert_eq!(gps_day("estimated_oxygen_variation-2026-05-23.csv"), None);
        assert_eq!(gps_day("gps_location_2026-13-01.csv"), None);
    }

    #[test]
    fn reads_an_exercise_log() {
        let json = br#"[{
            "logId": 77905184236,
            "activityName": "Outdoor Bike",
            "activityTypeId": 90001,
            "startTime": "07/11/26 14:55:42",
            "duration": 4334000,
            "activeDuration": 4332000,
            "hasGps": true,
            "tcxLink": "https://www.fitbit.com/x?export=tcx"
        }]"#;
        let logs = read_logs(json).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].log_id, 77905184236);
        assert_eq!(logs[0].name, "Outdoor Bike");
        assert!(logs[0].has_gps);
        // `activeDuration` wins: `duration` also counts the paused seconds,
        // which have no coordinates behind them.
        assert_eq!(logs[0].duration, Duration::milliseconds(4332000));
        assert_eq!(
            logs[0].start,
            NaiveDate::from_ymd_opt(2026, 7, 11)
                .unwrap()
                .and_hms_opt(14, 55, 42)
                .unwrap()
        );
    }

    #[test]
    fn a_malformed_start_time_names_the_log() {
        let json =
            br#"[{"logId": 7, "activityName": "Walk", "startTime": "2026-07-11T14:55:42Z"}]"#;
        let message = read_logs(json).unwrap_err().to_string();
        assert!(message.contains('7'), "{message}");
        assert!(message.contains("MM/DD/YY"), "{message}");
    }

    #[test]
    fn types_are_counted_by_whether_a_track_can_come_out() {
        let logs = vec![
            log(1, "Walk", "07/11/26 08:00:00", 30, true),
            log(2, "Walk", "07/12/26 08:00:00", 30, false),
            log(3, "Swim", "07/13/26 08:00:00", 30, false),
        ];
        let types = activity_types(&logs);
        assert_eq!(types[0].name, "Walk");
        assert_eq!((types[0].with_gps, types[0].without_gps), (1, 1));
        assert_eq!(types[1].name, "Swim");
        assert_eq!((types[1].with_gps, types[1].without_gps), (0, 1));
    }

    #[test]
    fn type_names_match_however_they_are_spelled() {
        assert_eq!(fold("Outdoor Bike"), fold("outdoor_bike"));
        assert_eq!(fold("Outdoor Bike"), fold("OUTDOOR-BIKE"));
        assert_ne!(fold("Bike"), fold("Outdoor Bike"));

        let types = activity_types(&[log(1, "Outdoor Bike", "07/11/26 08:00:00", 30, true)]);
        assert!(unmatched(&types, &["outdoor bike".into()]).is_empty());
        assert_eq!(unmatched(&types, &["hiking".into()]), vec!["hiking"]);
    }

    #[test]
    fn a_window_straddling_midnight_touches_both_days() {
        let log = log(1, "Outdoor Bike", "07/11/26 23:40:00", 40, true);
        let (start, end) = log.window(Duration::zero());
        assert_eq!(
            days_touched(start, end),
            vec![
                NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 12).unwrap(),
            ]
        );
    }

    #[test]
    fn a_window_inside_one_day_touches_only_it() {
        let log = log(1, "Walk", "07/11/26 08:00:00", 40, true);
        let (start, end) = log.window(Duration::zero());
        assert_eq!(days_touched(start, end).len(), 1);
    }

    /// The offset shifts the window, which is the whole point: a log written
    /// in New York local time has to be moved four hours to find its points.
    #[test]
    fn an_offset_moves_the_window_off_the_wall_clock() {
        let log = log(1, "Walk", "07/11/26 10:00:00", 30, true);
        let (start, _) = log.window(Duration::hours(-4));
        assert_eq!(start.to_rfc3339(), "2026-07-11T14:00:00+00:00");
    }

    #[test]
    fn candidates_try_utc_first_and_grow_outwards() {
        let candidates = candidate_offsets();
        assert_eq!(candidates[0], Duration::zero());
        assert_eq!(candidates[1], Duration::minutes(-15));
        assert!(candidates.contains(&Duration::hours(-4)));
        assert!(candidates.contains(&Duration::minutes(330))); // India, +5:30
    }

    #[test]
    fn a_day_file_keeps_the_recording_device() {
        let csv = b"timestamp,latitude,longitude,altitude,data source\n\
                    2026-07-11T11:30:17Z,41.393259,-81.743461,236.0,Google Health App\n\
                    2026-07-11T11:30:16Z,41.393280,-81.743452,236.0,Google Fitbit Air\n";
        let day = NaiveDate::from_ymd_opt(2026, 7, 11).unwrap();
        let fixes = read_day(csv, day).unwrap();
        assert_eq!(fixes.len(), 2);
        // Sorted by time, so the Fitbit row at :16 comes first despite being
        // second in the file.
        assert_eq!(fixes[0].source, "Google Fitbit Air");
        assert_eq!(fixes[0].point.elevation, Some(236.0));
    }

    /// Fractional seconds are in the real archive and are still RFC 3339.
    #[test]
    fn day_file_times_may_carry_fractional_seconds() {
        let csv = b"timestamp,latitude,longitude,altitude,data source\n\
                    2026-05-24T11:06:17.328Z,41.4,-81.7,236.0,Google Health App\n";
        let day = NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        assert_eq!(read_day(csv, day).unwrap().len(), 1);
    }

    #[test]
    fn the_busiest_device_wins_unless_one_is_named() {
        let make = |source: &str, second: u32| Fix {
            point: Point::new(1.0, 2.0).unwrap().with_time(Some(
                Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, second).unwrap(),
            )),
            source: source.to_string(),
        };
        let fixes = vec![make("Watch", 1), make("Phone", 2), make("Phone", 3)];

        let (kept, passed_over) = choose_source(&fixes, None);
        assert_eq!(kept, "Phone");
        assert_eq!(passed_over, vec!["Watch".to_string()]);

        let (kept, passed_over) = choose_source(&fixes, Some("watch"));
        assert_eq!(kept, "Watch");
        assert_eq!(passed_over, vec!["Phone".to_string()]);
    }
}
