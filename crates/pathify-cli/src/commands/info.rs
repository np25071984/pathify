use std::io::Write;

use anyhow::{Context, Result};
use pathify_core::formats;
use pathify_core::summary::Summary;

use crate::cli::InfoArgs;
use crate::io::read_input;

pub fn run(args: &InfoArgs, out: &mut dyn Write) -> Result<()> {
    let (bytes, path) = read_input(&args.input)?;
    let format = formats::detect(args.from, path.as_deref(), &bytes)?;
    let trace = formats::read(format, &bytes)?;
    let summary = Summary::of(&trace, args.elevation_threshold);

    if args.json {
        serde_json::to_writer_pretty(&mut *out, &summary).context("failed to write JSON")?;
        writeln!(out)?;
    } else {
        write_table(&summary, out)?;
    }
    Ok(())
}

fn write_table(summary: &Summary, out: &mut dyn Write) -> Result<()> {
    if let Some(name) = &summary.name {
        writeln!(out, "{name}")?;
    }

    let mut row = |label: &str, value: String| -> Result<()> {
        writeln!(out, "  {label:<12}{value}")?;
        Ok(())
    };

    if let Some(format) = summary.source_format {
        row("format", format.to_string())?;
    }
    row("tracks", thousands(summary.tracks))?;
    row("segments", thousands(summary.segments))?;
    row("points", thousands(summary.points))?;
    row("distance", format!("{:.2} km", summary.distance_m / 1000.0))?;

    if let Some(seconds) = summary.duration_s {
        row("duration", format_duration(seconds))?;
    }
    if let Some(speed) = summary.avg_speed_mps {
        row("avg speed", format!("{:.1} km/h", speed * 3.6))?;
    }
    if let Some(elevation) = &summary.elevation {
        row(
            "elevation",
            format!(
                "+{:.0} m / -{:.0} m  ({:.0}–{:.0} m)",
                elevation.gain_m, elevation.loss_m, elevation.min_m, elevation.max_m
            ),
        )?;
    }
    if let (Some(start), Some(end)) = (summary.start_time, summary.end_time) {
        row(
            "time",
            format!(
                "{} → {} UTC",
                start.format("%Y-%m-%d %H:%M:%S"),
                end.format("%Y-%m-%d %H:%M:%S")
            ),
        )?;
    }
    if let Some(bounds) = &summary.bounds {
        row(
            "bounds",
            format!(
                "{:.4}, {:.4} → {:.4}, {:.4}",
                bounds.min_lat, bounds.min_lon, bounds.max_lat, bounds.max_lon
            ),
        )?;
    }
    Ok(())
}

/// Format a whole number with thousands separators, so a point count stays
/// readable at the scale a day-long recording actually produces.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn format_duration(seconds: i64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(42), "42");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_204), "1,204");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn formats_durations_with_and_without_hours() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(61), "1:01");
        assert_eq!(format_duration(3_751), "1:02:31");
        assert_eq!(format_duration(36_000), "10:00:00");
    }
}
