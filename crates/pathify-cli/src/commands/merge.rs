use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result, anyhow};
use chrono::Duration;
use pathify_core::merge::{MergeMode, MergeOptions, MergeReport, merge};
use pathify_core::{Format, Trace, formats};

use crate::cli::MergeArgs;
use crate::io::read_input;

pub fn run(args: &MergeArgs, out: &mut dyn Write) -> Result<()> {
    let primary = args.primary_index().map_err(|message| anyhow!(message))?;

    let mut traces: Vec<Trace> = Vec::with_capacity(args.inputs.len());
    let mut first_format: Option<Format> = None;
    for input in &args.inputs {
        let (bytes, path) = read_input(input)?;
        let format = formats::detect(args.from, path.as_deref(), &bytes)?;
        first_format.get_or_insert(format);
        traces.push(formats::read(format, &bytes)?);
    }

    let options = MergeOptions {
        primary,
        dedup: !args.no_dedup,
        window: args.dedup_window.map(seconds),
        radius_m: args.dedup_radius,
        segment_gap: seconds(args.segment_gap),
    };
    let (merged, report) = merge(&traces, &options);

    // Default to the first input's format so a merge of GPX files stays GPX.
    let target = args
        .to
        .or_else(|| args.output.as_deref().and_then(Format::from_path))
        .or(first_format)
        .unwrap_or(Format::Gpx);

    match &args.output {
        Some(destination) => {
            let file = File::create(destination)
                .with_context(|| format!("failed to create {}", destination.display()))?;
            let mut writer = BufWriter::new(file);
            formats::write(target, &merged, &mut writer)?;
            writer
                .flush()
                .with_context(|| format!("failed to write {}", destination.display()))?;
        }
        None => formats::write(target, &merged, out)?,
    }

    if args.verbose {
        // Diagnostics go to stderr so the merged trace on stdout stays pipeable.
        eprintln!("{}", describe(&report));
    }
    Ok(())
}

fn seconds(value: f64) -> Duration {
    Duration::milliseconds((value * 1000.0).round() as i64)
}

fn describe(report: &MergeReport) -> String {
    match report.mode {
        MergeMode::Concatenation => format!(
            "concatenated {} inputs: {} points (no overlap in time, nothing matched)",
            report.inputs, report.output_points
        ),
        MergeMode::Reconciliation => format!(
            "reconciled {} inputs within {:.1}s: {} points in, {} matched as duplicates, {} out",
            report.inputs,
            report.window_s.unwrap_or_default(),
            report.input_points,
            report.matched_points,
            report.output_points
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_fractional_seconds_to_a_duration() {
        assert_eq!(seconds(1.5), Duration::milliseconds(1500));
        assert_eq!(seconds(120.0), Duration::seconds(120));
    }

    #[test]
    fn describes_each_mode_in_its_own_terms() {
        let concatenated = MergeReport {
            mode: MergeMode::Concatenation,
            inputs: 2,
            input_points: 10,
            output_points: 10,
            matched_points: 0,
            window_s: None,
        };
        let text = describe(&concatenated);
        assert!(text.contains("concatenated 2 inputs"), "{text}");
        assert!(text.contains("no overlap"), "{text}");

        let reconciled = MergeReport {
            mode: MergeMode::Reconciliation,
            inputs: 2,
            input_points: 16,
            output_points: 10,
            matched_points: 6,
            window_s: Some(15.0),
        };
        let text = describe(&reconciled);
        assert!(text.contains("6 matched"), "{text}");
        assert!(text.contains("15.0s"), "{text}");
    }
}
