use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result};
use pathify_core::clean::{CleanOptions, CleanReport, clean};
use pathify_core::spatial::privacy::Geofence;
use pathify_core::{Format, Point, formats};

use crate::cli::CleanArgs;
use crate::io::read_input;

pub fn run(args: &CleanArgs, out: &mut dyn Write) -> Result<()> {
    let (bytes, path) = read_input(args.input.as_deref())?;
    let source = formats::detect(args.from, path.as_deref(), &bytes)?;
    let trace = formats::read(source, &bytes)?;

    let mut fences = Vec::with_capacity(args.redact_around.len());
    for fence in &args.redact_around {
        let center = Point::new(fence.lat, fence.lon)?;
        fences.push(Geofence::new(center, fence.radius_m));
    }

    let options = CleanOptions {
        drift: !args.no_drift_filter,
        max_speed_mps: args.max_speed,
        fences,
        trim_ends_m: args.trim_ends,
    };
    let (cleaned, report) = clean(&trace, &options);

    let target = args
        .to
        .or_else(|| args.output.as_deref().and_then(Format::from_path))
        .unwrap_or(source);

    match &args.output {
        Some(destination) => {
            let file = File::create(destination)
                .with_context(|| format!("failed to create {}", destination.display()))?;
            let mut writer = BufWriter::new(file);
            formats::write(target, &cleaned, &mut writer)?;
            writer
                .flush()
                .with_context(|| format!("failed to write {}", destination.display()))?;
        }
        None => formats::write(target, &cleaned, out)?,
    }

    if args.verbose {
        eprintln!("{}", describe(&report));
    }
    Ok(())
}

fn describe(report: &CleanReport) -> String {
    let mut parts = Vec::new();
    if report.drift_removed > 0 {
        parts.push(format!("{} bad fixes", report.drift_removed));
    }
    if report.redacted_removed > 0 {
        parts.push(format!("{} inside a fence", report.redacted_removed));
    }
    if report.trimmed_removed > 0 {
        parts.push(format!("{} near the ends", report.trimmed_removed));
    }

    if parts.is_empty() {
        return format!("kept all {} points; nothing to remove", report.points_after);
    }
    format!(
        "removed {} of {} points ({}); {} remain",
        report.points_before - report.points_after,
        report.points_before,
        parts.join(", "),
        report.points_after
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(drift: usize, redacted: usize, trimmed: usize) -> CleanReport {
        CleanReport {
            points_before: 100,
            points_after: 100 - drift - redacted - trimmed,
            drift_removed: drift,
            redacted_removed: redacted,
            trimmed_removed: trimmed,
        }
    }

    #[test]
    fn describes_each_kind_of_removal_separately() {
        let text = describe(&report(2, 5, 3));
        assert!(text.contains("removed 10 of 100"), "{text}");
        assert!(text.contains("2 bad fixes"), "{text}");
        assert!(text.contains("5 inside a fence"), "{text}");
        assert!(text.contains("3 near the ends"), "{text}");
        assert!(text.contains("90 remain"), "{text}");
    }

    #[test]
    fn mentions_only_what_actually_happened() {
        let text = describe(&report(2, 0, 0));
        assert!(text.contains("2 bad fixes"), "{text}");
        assert!(!text.contains("fence"), "{text}");
        assert!(!text.contains("ends"), "{text}");
    }

    #[test]
    fn says_so_plainly_when_nothing_was_removed() {
        let text = describe(&report(0, 0, 0));
        assert!(text.contains("kept all 100 points"), "{text}");
    }
}
