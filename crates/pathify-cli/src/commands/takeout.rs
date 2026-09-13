//! `pathify takeout` — GPS traces out of a Google Takeout archive.
//!
//! The archive reading and the join live in `pathify_core::takeout`. What is
//! here is the part that needs a terminal or a user: deciding which activity
//! types were asked for, and saying on stderr what could not be found.

use std::fs::File;
use std::io::{BufWriter, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Duration;
use pathify_core::takeout::{Activity, Archive, Backend, Catalog, Options, Report, fitbit};
use pathify_core::{Format, formats};
use serde::Serialize;

use crate::cli::TakeoutArgs;

pub fn run(args: &TakeoutArgs, out: &mut dyn Write) -> Result<()> {
    let mut archive = Archive::open(&args.archives)?;

    match archive.backend() {
        Some(Backend::Fitbit) => {}
        Some(Backend::Timeline) => bail!(
            "this export holds Location History (Timeline) data, which `takeout` \
             cannot read yet.\n\
             It reads Google Health / Fitbit exports: the ones with a \
             `Physical Activity_GoogleData` folder."
        ),
        // Naming what is in there matters: the fix is a different export at
        // takeout.google.com, and no amount of re-running this will find
        // location data that was never ticked.
        None => bail!(
            "no location data in {}.\n\
             `takeout` reads Google Health / Fitbit exports. This archive holds: {}.\n\
             Re-export from takeout.google.com with Google Health (or Location \
             History) selected.",
            args.archives
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            list_or_nothing(&archive.products()),
        ),
    }

    let catalog = Catalog::of(&mut archive)?;
    let types = catalog.types();
    if types.is_empty() {
        bail!("no exercise logs in this archive, so there is nothing to select from");
    }

    if args.list {
        if args.json {
            print_types_as_json(&catalog, &types, out)?;
        } else {
            print_types(&catalog, &types, out)?;
        }
        return Ok(());
    }

    let chosen = match &args.r#type {
        Some(requested) => {
            let unknown = fitbit::unmatched(&types, requested);
            if !unknown.is_empty() {
                bail!(
                    "no activity type called {} in this archive.\nIt has: {}.",
                    unknown
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    types
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            requested.clone()
        }
        None => ask(&catalog, &types)?,
    };

    let options = Options {
        types: chosen,
        source: args.source.clone(),
        segment_gap: seconds(args.segment_gap),
        offset: None,
    };
    let target = args.target_format();

    if args.per_activity {
        // clap enforces the pairing; this is only the type falling out of it.
        let directory = args
            .output
            .as_deref()
            .ok_or_else(|| anyhow!("--per-activity needs --output to name a directory"))?;

        let (activities, report) =
            pathify_core::takeout::extract_each(&mut archive, &catalog, &options)?;
        if args.verbose {
            report_to_stderr(&report);
        }
        if activities.is_empty() {
            bail!(nothing_came_out(&report));
        }
        return write_each(&activities, target, directory);
    }

    let (trace, report) = pathify_core::takeout::extract(&mut archive, &catalog, &options)?;

    if args.verbose {
        report_to_stderr(&report);
    }
    if trace.is_empty() {
        bail!(nothing_came_out(&report));
    }

    match &args.output {
        Some(destination) => write_file(&trace, target, destination)?,
        None => formats::write(target, &trace, out)?,
    }
    Ok(())
}

/// Why an extraction that selected something still has nothing to hand over.
fn nothing_came_out(report: &Report) -> String {
    format!(
        "the {} selected {} produced no points. Re-run with --verbose to see why.",
        report.selected,
        if report.selected == 1 {
            "activity"
        } else {
            "activities"
        }
    )
}

fn write_file(trace: &pathify_core::Trace, target: Format, destination: &Path) -> Result<()> {
    let file = File::create(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let mut writer = BufWriter::new(file);
    formats::write(target, trace, &mut writer)?;
    writer
        .flush()
        .with_context(|| format!("failed to write {}", destination.display()))
}

/// Write one file per activity into `directory`.
///
/// The directory is created if it is not there, because the caller asking for
/// a batch of files has no other reason to have made one. Existing files of
/// the same name are overwritten: the names come from the activities, so a
/// second run over the same archive rewrites its own output rather than
/// piling a second copy of it alongside.
fn write_each(activities: &[Activity], target: Format, directory: &Path) -> Result<()> {
    if directory.exists() && !directory.is_dir() {
        bail!(
            "--per-activity writes a file per activity, so --output has to name a \
             directory; {} is a file",
            directory.display()
        );
    }
    std::fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    for (activity, name) in activities.iter().zip(file_names(activities, target)) {
        write_file(&activity.trace, target, &directory.join(name))?;
    }
    Ok(())
}

/// The file name for each activity, in the order they were extracted.
///
/// `20260711T113000Z-outdoor-bike.gpx`: the activity's own start in UTC, then
/// its type. Start first so that a listing of the directory is in the order
/// the outings happened, and compact ISO 8601 because a colon cannot appear
/// in a filename on Windows.
///
/// Two logs of one type can start in the same second — a phone and a watch
/// that each filed the same walk, most often — so a name that would be
/// written twice takes the log id as well. Only the colliding pair is
/// lengthened, and which one gets which name does not depend on the order
/// they were extracted in.
fn file_names(activities: &[Activity], target: Format) -> Vec<String> {
    let stems: Vec<String> = activities
        .iter()
        .map(|activity| {
            format!(
                "{}-{}",
                activity.start.format("%Y%m%dT%H%M%SZ"),
                slug(&activity.name)
            )
        })
        .collect();

    stems
        .iter()
        .enumerate()
        .map(|(index, stem)| {
            let shared = stems
                .iter()
                .enumerate()
                .any(|(other, name)| other != index && name == stem);
            let extension = target.name();
            if shared {
                format!("{stem}-{}.{extension}", activities[index].log_id)
            } else {
                format!("{stem}.{extension}")
            }
        })
        .collect()
}

/// An activity type as a filename fragment: `Outdoor Bike` to `outdoor-bike`.
fn slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    // A name of nothing but punctuation would otherwise produce a file called
    // `.gpx`, which is a hidden file with no name at all.
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "activity".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Put the choice to the user, or explain why it cannot be put.
fn ask(catalog: &Catalog, types: &[fitbit::ActivityType]) -> Result<Vec<String>> {
    // Both ends have to be a terminal. A piped stdout means the trace is on
    // its way somewhere — `pathify takeout a.zip | pathify view` — and a menu
    // would paint over the screen and corrupt the pipe; a piped stdin means
    // there is nobody at a keyboard to answer it.
    if !(pathify_tui::is_interactive() && std::io::stdin().is_terminal()) {
        bail!(
            "no activity type chosen, and there is no terminal to ask at.\n\
             Pass --type, e.g. `--type \"walk,outdoor bike\"`, or run \
             `--list` to see what this archive holds."
        );
    }

    let choices = types
        .iter()
        .map(|activity| {
            pathify_tui::Choice::new(&activity.name, counts(activity))
                // A type with no GPS at all cannot become a track. It is
                // still listed, so the user can see it was found rather than
                // wonder where their swims went.
                .enabled(activity.with_gps > 0)
        })
        .collect();

    let title = format!(
        "{} activity logs, {} with GPS, across {} days of recording.",
        catalog.logs.len(),
        catalog.logs_with_gps(),
        catalog.day_count(),
    );

    let Some(picked) = pathify_tui::choose(&title, choices)? else {
        bail!("cancelled");
    };
    if picked.is_empty() {
        bail!("no activity types selected, so there is nothing to extract");
    }
    Ok(picked.into_iter().map(|i| types[i].name.clone()).collect())
}

/// Print the activity types as a table.
///
/// With a header row, because a bare pair of numbers leaves the reader to
/// supply the noun: `(117 with GPS, 27 without)` reads as "27 without *what*".
/// The first column is named after the flag that consumes it, since telling
/// you what to pass to `--type` is the whole job of `--list`.
fn print_types(
    catalog: &Catalog,
    types: &[fitbit::ActivityType],
    out: &mut dyn Write,
) -> Result<()> {
    const COLUMNS: [&str; 3] = ["activity type", "logs", "with GPS"];

    let widest = |header: &str, cell: fn(&fitbit::ActivityType) -> usize| {
        types
            .iter()
            .map(&cell)
            .max()
            .unwrap_or(0)
            .max(header.chars().count())
    };
    let name = widest(COLUMNS[0], |a| a.name.chars().count());
    let logs = widest(COLUMNS[1], |a| a.total().to_string().len());
    let gps = widest(COLUMNS[2], |a| a.with_gps.to_string().len());

    writeln!(
        out,
        "{:name$}  {:>logs$}  {:>gps$}",
        COLUMNS[0], COLUMNS[1], COLUMNS[2]
    )?;
    for activity in types {
        writeln!(
            out,
            "{:name$}  {:>logs$}  {:>gps$}",
            activity.name,
            activity.total(),
            activity.with_gps
        )?;
    }
    writeln!(
        out,
        "\n{} logs, {} with GPS, {} days of recording",
        catalog.logs.len(),
        catalog.logs_with_gps(),
        catalog.day_count()
    )?;
    Ok(())
}

/// The same listing as JSON, for a program driving `takeout` rather than a
/// person reading it.
///
/// Shaped like `info --json`: snake_case keys, pretty-printed, one trailing
/// newline. The per-type rows carry the two columns the table shows and no
/// derived third — a caller that wants the logs without GPS can subtract, and
/// two numbers that must agree are better than three.
fn print_types_as_json(
    catalog: &Catalog,
    types: &[fitbit::ActivityType],
    out: &mut dyn Write,
) -> Result<()> {
    let listing = ListJson {
        types: types
            .iter()
            .map(|activity| TypeJson {
                name: &activity.name,
                logs: activity.total(),
                with_gps: activity.with_gps,
            })
            .collect(),
        total_logs: catalog.logs.len(),
        total_with_gps: catalog.logs_with_gps(),
        days_of_recording: catalog.day_count(),
    };
    serde_json::to_writer_pretty(&mut *out, &listing).context("failed to write JSON")?;
    writeln!(out)?;
    Ok(())
}

#[derive(Serialize)]
struct ListJson<'a> {
    types: Vec<TypeJson<'a>>,
    total_logs: usize,
    total_with_gps: usize,
    days_of_recording: usize,
}

#[derive(Serialize)]
struct TypeJson<'a> {
    name: &'a str,
    logs: usize,
    with_gps: usize,
}

/// The counts beside a menu row.
///
/// Spelled as a fraction rather than as a pair, because the menu has no room
/// for a header to say what each number counts, and because "0 of 29" is the
/// answer to why a row cannot be ticked.
fn counts(activity: &fitbit::ActivityType) -> String {
    format!("{} of {} with GPS", activity.with_gps, activity.total())
}

/// Say what the extraction did and, more usefully, what it could not do.
fn report_to_stderr(report: &Report) {
    eprintln!(
        "pathify: {} logs, {} with GPS; {} selected, {} tracks, {} points",
        report.logs, report.logs_with_gps, report.selected, report.tracks, report.points
    );

    if report.offset_detected {
        eprintln!(
            "pathify: exercise logs read as {}",
            offset_name(report.offset_s)
        );
    } else if report.selected > 0 {
        // The wall clock in the logs carries no offset, so if no shift lands
        // on any recorded point the join has failed and the tracks below are
        // not evidence of anything.
        eprintln!(
            "pathify: could not work out how the logs' clock relates to UTC; \
             assumed UTC"
        );
    }

    for (log, day) in &report.missing_days {
        eprintln!("pathify: log {log} claims GPS but there is no day file for {day}");
    }
    if !report.empty_windows.is_empty() {
        eprintln!(
            "pathify: {} log(s) claim GPS but their day file held no points in \
             their window: {}",
            report.empty_windows.len(),
            report
                .empty_windows
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for (log, kept, passed_over) in &report.source_choices {
        eprintln!(
            "pathify: log {log} was recorded by {} devices; kept `{kept}`, passed over {}",
            passed_over.len() + 1,
            passed_over
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

/// The detected offset as a person would write it.
fn offset_name(seconds: i64) -> String {
    if seconds == 0 {
        return "UTC".to_string();
    }
    let sign = if seconds < 0 { '-' } else { '+' };
    let (hours, minutes) = (seconds.abs() / 3600, (seconds.abs() % 3600) / 60);
    format!("UTC{sign}{hours:02}:{minutes:02}")
}

/// Turn the CLI's seconds into the `chrono::Duration` core works in, keeping
/// sub-second precision rather than truncating `--segment-gap 1.5s` to one.
fn seconds(value: f64) -> Duration {
    Duration::milliseconds((value * 1000.0).round() as i64)
}

fn list_or_nothing(products: &[String]) -> String {
    if products.is_empty() {
        "nothing at all".to_string()
    } else {
        products.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use pathify_core::model::Trace;

    fn activity(log_id: i64, name: &str, start: &str) -> Activity {
        Activity {
            log_id,
            name: name.to_string(),
            start: NaiveDateTime::parse_from_str(start, "%Y-%m-%d %H:%M:%S")
                .expect("a test timestamp should parse")
                .and_utc(),
            source: "Phone".to_string(),
            trace: Trace::default(),
        }
    }

    #[test]
    fn a_file_is_named_after_the_activitys_own_start_and_type() {
        let activities = [
            activity(1, "Walk", "2026-07-11 11:30:00"),
            activity(2, "Outdoor Bike", "2026-07-11 23:55:00"),
        ];
        assert_eq!(
            file_names(&activities, Format::Gpx),
            [
                "20260711T113000Z-walk.gpx",
                "20260711T235500Z-outdoor-bike.gpx"
            ]
        );
        assert_eq!(
            file_names(&activities, Format::GeoJson)[0],
            "20260711T113000Z-walk.geojson"
        );
    }

    /// Two of a type on one day are two files, because the name carries the
    /// time of day and not just the date.
    #[test]
    fn two_walks_on_one_day_do_not_collide() {
        let activities = [
            activity(1, "Walk", "2026-07-11 08:00:00"),
            activity(2, "Walk", "2026-07-11 18:20:00"),
        ];
        let names = file_names(&activities, Format::Gpx);
        assert_eq!(names[0], "20260711T080000Z-walk.gpx");
        assert_eq!(names[1], "20260711T182000Z-walk.gpx");
    }

    /// Same type, same second — a phone and a watch that each filed the same
    /// walk. The log id is what tells them apart, and only the pair that
    /// needs it carries one.
    #[test]
    fn two_logs_of_one_type_in_one_second_take_their_log_ids() {
        let activities = [
            activity(11, "Walk", "2026-07-11 08:00:00"),
            activity(12, "Walk", "2026-07-11 08:00:00"),
            activity(13, "Walk", "2026-07-11 09:00:00"),
        ];
        assert_eq!(
            file_names(&activities, Format::Gpx),
            [
                "20260711T080000Z-walk-11.gpx",
                "20260711T080000Z-walk-12.gpx",
                "20260711T090000Z-walk.gpx"
            ]
        );
    }

    #[test]
    fn a_type_name_becomes_one_lower_case_fragment() {
        assert_eq!(slug("Walk"), "walk");
        assert_eq!(slug("Outdoor Bike"), "outdoor-bike");
        assert_eq!(slug("Rowing  machine"), "rowing-machine");
        assert_eq!(slug("Sport/Other"), "sport-other");
        // Never a file called `.gpx`, which has no name at all.
        assert_eq!(slug("///"), "activity");
    }

    #[test]
    fn an_offset_is_named_the_way_a_person_writes_one() {
        assert_eq!(offset_name(0), "UTC");
        assert_eq!(offset_name(-4 * 3600), "UTC-04:00");
        assert_eq!(offset_name(5 * 3600 + 30 * 60), "UTC+05:30");
    }

    #[test]
    fn a_fractional_gap_survives_the_trip_into_core() {
        assert_eq!(seconds(1.5), Duration::milliseconds(1500));
        assert_eq!(seconds(120.0), Duration::seconds(120));
    }
}
