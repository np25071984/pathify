use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use pathify_core::Format;
use pathify_core::spatial::{DEFAULT_NOISE_THRESHOLD_M, MAX_PLAUSIBLE_SPEED_MPS};

/// A local-first toolkit for GPS trace data.
#[derive(Debug, Parser)]
#[command(
    name = "pathify",
    version,
    about = "A local-first CLI for GPS trace data.",
    long_about = "Inspect, clean, merge, convert, and view GPS traces.\n\n\
                  Every command reads a file or `-` for stdin and writes to stdout, \
                  so they compose in a pipeline. Nothing is ever uploaded anywhere."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report distance, elevation, duration, and extent of a trace.
    Info(InfoArgs),

    /// Convert a trace between formats.
    Convert(ConvertArgs),

    /// Combine several traces into one.
    ///
    /// Inputs that overlap in time are reconciled: points recording the same
    /// moment on different devices are matched and collapsed, so the merged
    /// trace does not report double the distance.
    ///
    /// Inputs that do not overlap are simply ordered and stitched together,
    /// keeping their own segment boundaries.
    Merge(MergeArgs),

    /// Remove bad fixes, and locations you do not want to share.
    ///
    /// Drift filtering runs by default: it only ever discards fixes that imply
    /// an impossible speed, which are readings the receiver got wrong.
    ///
    /// Redaction never runs unless you ask for it, because it deletes real
    /// places you went. Use --trim-ends to hide where a journey started and
    /// finished without naming the address, or --redact-around to fence a
    /// location you can name.
    Clean(CleanArgs),

    /// Draw a trace on an interactive terminal map.
    ///
    /// Unlike the other commands this one paints a screen instead of emitting a
    /// document, so it needs a terminal and cannot be piped into a file.
    /// Reading the trace itself from a pipe is fine: key presses come from the
    /// controlling terminal, not from stdin.
    ///
    /// Pan with the arrow keys or hjkl, zoom with + and -, press f to fit the
    /// whole trace, ? for help, and q to quit.
    View(ViewArgs),
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Input file. Omit it to read a piped trace, or pass `-` for stdin.
    #[arg(value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Emit JSON instead of a human-readable table.
    #[arg(long)]
    pub json: bool,

    /// Override input format detection.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,

    /// Ignore elevation changes smaller than this, in meters.
    ///
    /// GPS elevation jitters by a couple of meters at rest; without a floor,
    /// a flat ride reports hundreds of meters of climb.
    #[arg(long, value_name = "METERS", default_value_t = DEFAULT_NOISE_THRESHOLD_M)]
    pub elevation_threshold: f64,
}

#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// Input file. Omit it to read a piped trace, or pass `-` for stdin.
    #[arg(value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Target format. Optional when `--output` has a recognizable extension.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub to: Option<Format>,

    /// Override input format detection.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,

    /// Write to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

impl ConvertArgs {
    /// Resolve the target format from `--to`, falling back to the output
    /// file's extension so `convert ride.gpx -o ride.csv` needs no flag.
    pub fn target_format(&self) -> Result<Format, String> {
        if let Some(format) = self.to {
            return Ok(format);
        }
        if let Some(format) = self.output.as_deref().and_then(Format::from_path) {
            return Ok(format);
        }
        Err(
            "specify the target format with --to, or give --output a path whose \
             extension implies one"
                .to_string(),
        )
    }
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    /// Input files, or `-` for stdin. At least two to be worth merging.
    #[arg(required = true, value_name = "FILE", num_args = 1..)]
    pub inputs: Vec<PathBuf>,

    /// Output format. Defaults to the format of the first input.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub to: Option<Format>,

    /// Override input format detection, for every input.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,

    /// Write to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Input whose position, elevation, and time win where sources disagree.
    ///
    /// Defaults to the first input. Optional per-point extras such as heart
    /// rate are unioned across sources either way.
    #[arg(long, value_name = "FILE")]
    pub primary: Option<PathBuf>,

    /// Concatenate without matching, even where inputs overlap in time.
    #[arg(long)]
    pub no_dedup: bool,

    /// Override the matching time window, in seconds.
    #[arg(long, value_name = "SECONDS")]
    pub dedup_window: Option<f64>,

    /// Override the matching distance, in meters.
    #[arg(long, value_name = "METERS")]
    pub dedup_radius: Option<f64>,

    /// Gap that starts a new segment in reconciled output, in seconds.
    #[arg(long, value_name = "SECONDS", default_value_t = 120.0)]
    pub segment_gap: f64,

    /// Report what the merge did on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}

impl MergeArgs {
    /// Position of `--primary` among the inputs.
    ///
    /// Matched by path as given, so it has to name one of the inputs; silently
    /// falling back to the first would hand the user a merge that took the
    /// wrong source's coordinates without saying so.
    pub fn primary_index(&self) -> Result<usize, String> {
        let Some(primary) = &self.primary else {
            return Ok(0);
        };
        self.inputs
            .iter()
            .position(|input| input == primary)
            .ok_or_else(|| format!("--primary {} is not one of the inputs", primary.display()))
    }
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Input file. Omit it to read a piped trace, or pass `-` for stdin.
    #[arg(value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Output format. Defaults to the input's format.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub to: Option<Format>,

    /// Override input format detection.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,

    /// Write to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Keep fixes that imply an impossible speed.
    #[arg(long)]
    pub no_drift_filter: bool,

    /// Speed above which a step is treated as a bad fix, in meters per second.
    #[arg(long, value_name = "M/S", default_value_t = MAX_PLAUSIBLE_SPEED_MPS)]
    pub max_speed: f64,

    /// Remove everything within RADIUS meters of LAT,LON. Repeatable.
    // `allow_hyphen_values` because a southern-hemisphere latitude starts with
    // a minus sign, and without it clap reads the whole value as an unknown
    // flag — so the fence was unusable for half the planet, and
    // `parse_geofence` never ran to say why.
    #[arg(
        long,
        value_name = "LAT,LON,RADIUS",
        allow_hyphen_values = true,
        value_parser = parse_geofence
    )]
    pub redact_around: Vec<GeofenceArg>,

    /// Remove everything within this many meters of where the trace starts and
    /// ends, without having to name the location.
    #[arg(long, value_name = "METERS")]
    pub trim_ends: Option<f64>,

    /// Report what was removed, on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}

/// A `lat,lon,radius` triple from the command line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeofenceArg {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
}

fn parse_geofence(value: &str) -> Result<GeofenceArg, String> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    let [lat, lon, radius] = parts.as_slice() else {
        return Err(format!(
            "expected LAT,LON,RADIUS (three comma-separated numbers), got `{value}`"
        ));
    };

    let number = |text: &str, name: &str| -> Result<f64, String> {
        text.parse::<f64>()
            .map_err(|_| format!("`{text}` is not a number for {name}"))
    };

    let (lat, lon, radius_m) = (
        number(lat, "latitude")?,
        number(lon, "longitude")?,
        number(radius, "radius")?,
    );

    if !(-90.0..=90.0).contains(&lat) {
        return Err(format!("latitude {lat} is outside -90..=90"));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(format!("longitude {lon} is outside -180..=180"));
    }
    // A negative radius would silently fence nothing, quietly publishing the
    // location the user asked to hide.
    if radius_m < 0.0 || !radius_m.is_finite() {
        return Err(format!("radius {radius_m} must be zero or more"));
    }

    Ok(GeofenceArg { lat, lon, radius_m })
}

#[derive(Debug, Args)]
pub struct ViewArgs {
    /// Input file. Omit it to read a piped trace, or pass `-` for stdin.
    #[arg(value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Override input format detection.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,
}

fn parse_format(value: &str) -> Result<Format, String> {
    value.parse::<Format>().map_err(|_| {
        let names: Vec<&str> = Format::ALL.iter().map(|f| f.name()).collect();
        format!(
            "unknown format `{value}` (expected one of: {})",
            names.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::path::Path;

    fn info_args(argv: &[&str]) -> InfoArgs {
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Info(args) => args,
            other => panic!("expected `info`, parsed {other:?}"),
        }
    }

    fn convert_args(argv: &[&str]) -> ConvertArgs {
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Convert(args) => args,
            other => panic!("expected `convert`, parsed {other:?}"),
        }
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// A latitude south of the equator starts with a minus sign, which clap
    /// reads as the start of another flag unless told otherwise. Without
    /// `allow_hyphen_values` this fails with "unexpected argument '-3'" and
    /// `parse_geofence` never runs, so redaction by coordinate was unusable
    /// across the whole southern hemisphere.
    #[test]
    fn a_southern_hemisphere_fence_is_a_value_not_a_flag() {
        let parsed = Cli::try_parse_from([
            "pathify",
            "clean",
            "ride.gpx",
            "--redact-around",
            "-33.8688,151.2093,300",
        ])
        .expect("a negative latitude should parse");

        let Command::Clean(args) = parsed.command else {
            panic!("expected `clean`");
        };
        assert_eq!(
            args.redact_around,
            vec![GeofenceArg {
                lat: -33.8688,
                lon: 151.2093,
                radius_m: 300.0,
            }]
        );
    }

    /// The same for a fence at negative latitude *and* longitude, which is
    /// most of South America and the South Atlantic.
    #[test]
    fn a_fence_negative_on_both_axes_parses() {
        let parsed = Cli::try_parse_from([
            "pathify",
            "clean",
            "ride.gpx",
            "--redact-around",
            "-34.6037,-58.3816,500",
        ])
        .expect("a fence negative on both axes should parse");

        let Command::Clean(args) = parsed.command else {
            panic!("expected `clean`");
        };
        assert_eq!(args.redact_around[0].lat, -34.6037);
        assert_eq!(args.redact_around[0].lon, -58.3816);
    }

    /// Omitting the file leaves it unset rather than defaulting to `-`, which
    /// is what lets the command tell "you forgot the filename" apart from "you
    /// asked for stdin".
    #[test]
    fn omitting_the_file_leaves_the_input_unset() {
        let args = info_args(&["pathify", "info"]);
        assert_eq!(args.input, None);
        assert!(!args.json);
        assert_eq!(args.from, None);
    }

    #[test]
    fn info_accepts_a_path_and_flags() {
        let args = info_args(&[
            "pathify",
            "info",
            "ride.gpx",
            "--json",
            "--from",
            "gpx",
            "--elevation-threshold",
            "5",
        ]);
        assert_eq!(args.input.as_deref(), Some(Path::new("ride.gpx")));
        assert!(args.json);
        assert_eq!(args.from, Some(Format::Gpx));
        assert_eq!(args.elevation_threshold, 5.0);
    }

    #[test]
    fn an_unknown_format_is_rejected_with_the_valid_names() {
        let err = Cli::try_parse_from(["pathify", "info", "--from", "kmz"]).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("kmz"), "{message}");
        assert!(message.contains("geojson"), "{message}");
    }

    #[test]
    fn convert_takes_the_target_format_from_the_to_flag() {
        let args = convert_args(&["pathify", "convert", "ride.gpx", "--to", "csv"]);
        assert_eq!(args.target_format().unwrap(), Format::Csv);
        assert_eq!(args.output, None);
    }

    /// `convert ride.gpx -o ride.csv` should not also need `--to csv`.
    #[test]
    fn convert_infers_the_target_from_the_output_extension() {
        let args = convert_args(&["pathify", "convert", "ride.gpx", "-o", "out/ride.geojson"]);
        assert_eq!(args.target_format().unwrap(), Format::GeoJson);
    }

    #[test]
    fn an_explicit_to_flag_beats_the_output_extension() {
        let args = convert_args(&[
            "pathify",
            "convert",
            "r.gpx",
            "--to",
            "csv",
            "-o",
            "r.geojson",
        ]);
        assert_eq!(args.target_format().unwrap(), Format::Csv);
    }

    #[test]
    fn convert_without_a_resolvable_target_says_how_to_fix_it() {
        let args = convert_args(&["pathify", "convert", "ride.gpx"]);
        let message = args.target_format().unwrap_err();
        assert!(message.contains("--to"), "{message}");
        assert!(message.contains("--output"), "{message}");
    }
}
