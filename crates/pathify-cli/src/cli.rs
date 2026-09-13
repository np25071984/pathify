use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use pathify_core::Format;

use crate::units::{parse_distance, parse_duration, parse_speed};

/// Defaults as the command line spells them, unit and all, so `--help` shows
/// a value you could have typed yourself. The tests below keep each one in
/// step with the constant it stands for.
const DEFAULT_ELEVATION_THRESHOLD: &str = "3m";
const DEFAULT_MAX_SPEED: &str = "60m/s";
const DEFAULT_SEGMENT_GAP: &str = "120s";

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

    /// Pull GPS traces out of a Google Takeout archive.
    ///
    /// Make the export at takeout.google.com: deselect everything, tick
    /// Google Health, and pass the Zip here without unpacking it. Repeat the
    /// argument for a multi-part export. The README has the full steps.
    ///
    /// The unit of selection is the activity type — `Walk`, `Outdoor Bike` —
    /// not the file, because an archive holds hundreds of recordings spread
    /// across per-day files that do not correspond to activities at all.
    ///
    /// Nothing is unzipped: only the exercise logs and the GPS day files are
    /// ever read, and only for the days the chosen activities fall on. A
    /// Takeout export carries sleep, heart rate and other health data
    /// alongside the locations; none of it is read, none of it is copied
    /// anywhere, and the archive itself is never modified.
    ///
    /// The first and last points of these tracks are usually a home address.
    /// `pathify clean --trim-ends` and `--redact-around` are the way to deal
    /// with that before sharing anything.
    Takeout(TakeoutArgs),

    /// Draw a trace onto a map image you already have, as a PNG.
    ///
    /// Pathify never downloads the map: it has no network code and is not
    /// going to get any. Get the area from `pathify info`'s bbox row, fetch a
    /// PNG of it yourself, and pass it here.
    ///
    /// Two modes. By default the track is stroked over the map. With --fog the
    /// map is darkened instead and cleared again along the route, so the only
    /// map you can read is the ground you actually covered.
    Render(RenderArgs),
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

    /// Ignore elevation changes smaller than this, e.g. `3m` or `10ft`.
    ///
    /// GPS elevation jitters by a couple of meters at rest; without a floor,
    /// a flat ride reports hundreds of meters of climb.
    #[arg(
        long,
        value_name = "DISTANCE",
        value_parser = parse_distance,
        default_value = DEFAULT_ELEVATION_THRESHOLD
    )]
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

    /// Override the matching time window, e.g. `15s` or `2min`.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    pub dedup_window: Option<f64>,

    /// Override the matching distance, e.g. `15m` or `50ft`.
    #[arg(long, value_name = "DISTANCE", value_parser = parse_distance)]
    pub dedup_radius: Option<f64>,

    /// Gap that starts a new segment in reconciled output, e.g. `120s`.
    #[arg(
        long,
        value_name = "DURATION",
        value_parser = parse_duration,
        default_value = DEFAULT_SEGMENT_GAP
    )]
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

    /// Speed above which a step is treated as a bad fix, e.g. `60m/s` or
    /// `216km/h`.
    #[arg(
        long,
        value_name = "SPEED",
        value_parser = parse_speed,
        default_value = DEFAULT_MAX_SPEED
    )]
    pub max_speed: f64,

    /// Remove everything within RADIUS of LAT,LON. Repeatable.
    ///
    /// The radius carries its own unit, e.g. `47.65,-122.31,300m` or
    /// `47.65,-122.31,500ft`; a bare number is meters.
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

    /// Remove everything within this distance of where the trace starts and
    /// ends, without having to name the location, e.g. `300m` or `200ft`.
    #[arg(long, value_name = "DISTANCE", value_parser = parse_distance)]
    pub trim_ends: Option<f64>,

    /// Report what was removed, on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct TakeoutArgs {
    /// Takeout archive(s). Repeat for a multi-part export, or name the
    /// directory they were unzipped into.
    #[arg(required = true, value_name = "ARCHIVE", num_args = 1..)]
    pub archives: Vec<PathBuf>,

    /// Filter by activity type (comma-separated, e.g. "walk,outdoor bike").
    /// Case- and space-insensitive. Omit to choose from a menu.
    // `r#type`, not `type_filter`: the field name is what clap derives the
    // flag from, and `--type-filter` would match none of the documented
    // examples. There is no short form, because `-t` cannot also belong to
    // `--to`, and every other subcommand keeps `--to` long-only.
    #[arg(long, value_name = "TYPES", value_delimiter = ',')]
    pub r#type: Option<Vec<String>>,

    /// List the activity types in the archive and exit.
    #[arg(long, conflicts_with = "type")]
    pub list: bool,

    /// Emit `--list` as JSON instead of a table.
    ///
    /// Only `--list` has anything to say in JSON: everything else this
    /// command produces is a trace, and a trace has its own formats.
    #[arg(long, requires = "list")]
    pub json: bool,

    /// Keep only this recording device where several logged the same journey.
    ///
    /// Most days carry two — a phone and a watch — recording the same journey
    /// a few meters apart. Concatenating them would report twice the distance,
    /// so one is kept: the one that saw the most of the activity, unless this
    /// names another.
    #[arg(long, value_name = "NAME")]
    pub source: Option<String>,

    /// Gap that starts a new segment, e.g. `120s` or `5min`.
    #[arg(
        long,
        value_name = "DURATION",
        value_parser = parse_duration,
        default_value = DEFAULT_SEGMENT_GAP
    )]
    pub segment_gap: f64,

    /// Output format. Defaults to GPX, since a Zip implies no format of its own.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub to: Option<Format>,

    /// Write to this file instead of stdout.
    ///
    /// With --per-activity this names a directory instead, and is required.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Write one file per activity into the --output directory.
    ///
    /// The default welds every match into one trace, which is what you want
    /// to look at. A program importing each outing as its own record wants
    /// one file each; this writes them, named after the activity's own start
    /// time and type, into a directory created if it does not exist.
    // An explicit flag rather than "-o names a directory": what that meant
    // would depend on whether the path happened to exist already, so the same
    // command would combine or split depending on the state of the disk.
    #[arg(long, requires = "output", conflicts_with = "list")]
    pub per_activity: bool,

    /// Report what was found and skipped, on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}

impl TakeoutArgs {
    /// Resolve the output format.
    ///
    /// A deliberate departure from [`ConvertArgs::target_format`], which
    /// errors rather than guessing: there an input format exists to inherit,
    /// and here the input is a Zip, which implies nothing. So the fallback is
    /// GPX. `--to` still beats an `--output` extension, as it does everywhere
    /// else in this CLI — a flag the user typed should not lose to one they
    /// only implied.
    /// With --per-activity the output path is a directory, and a directory
    /// name implies no more about a format than the Zip does, so only `--to`
    /// and the GPX fallback are left.
    pub fn target_format(&self) -> Format {
        self.to
            .or_else(|| {
                self.output
                    .as_deref()
                    .filter(|_| !self.per_activity)
                    .and_then(Format::from_path)
            })
            .unwrap_or(Format::Gpx)
    }
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

    // The radius is a distance like any other flag's, so it takes a unit:
    // `47.65,-122.31,500ft`. `parse_distance` also rejects a negative radius,
    // which would silently fence nothing and quietly publish the location the
    // user asked to hide.
    let (lat, lon, radius_m) = (
        number(lat, "latitude")?,
        number(lon, "longitude")?,
        parse_distance(radius)?,
    );

    if !(-90.0..=90.0).contains(&lat) {
        return Err(format!("latitude {lat} is outside -90..=90"));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(format!("longitude {lon} is outside -180..=180"));
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

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// Trace file. Omit it to read a piped trace, or pass `-` for stdin.
    #[arg(value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Basemap PNG. Omit it to read a piped image.
    #[arg(long, value_name = "FILE")]
    pub basemap: Option<PathBuf>,

    /// Area the basemap covers. Defaults to the trace's own bounds, which is
    /// right when the image was downloaded for `pathify info`'s bbox.
    // `allow_hyphen_values` because half the world's longitudes are negative:
    // without it `--bbox -122.3,47.6,-122.2,47.7` is read as an unknown flag,
    // which makes the command unusable west of Greenwich.
    #[arg(
        long,
        value_name = "MIN_LON,MIN_LAT,MAX_LON,MAX_LAT",
        allow_hyphen_values = true,
        value_parser = parse_bbox
    )]
    pub bbox: Option<BboxArg>,

    /// Darken the map and clear it again only along the route.
    #[arg(long)]
    pub fog: bool,

    /// How far either side of the route the fog lifts, e.g. `6m` or `20ft`.
    /// Defaults to `6m`, or `20ft` where the locale prefers imperial.
    #[arg(
        long,
        value_name = "DISTANCE",
        value_parser = parse_distance,
        requires = "fog"
    )]
    pub reveal: Option<f64>,

    /// How dark the fog is, from 0 (none) to 1 (black). Defaults to 0.65.
    #[arg(long, value_name = "0..1", requires = "fog")]
    pub fog_opacity: Option<f64>,

    /// Colour of the drawn track.
    #[arg(long, value_name = "#RRGGBB", conflicts_with = "fog", value_parser = parse_hex_color)]
    pub track_color: Option<[u8; 3]>,

    /// Stroke width of the drawn track, in pixels.
    #[arg(long, value_name = "PX", conflicts_with = "fog")]
    pub track_width: Option<f64>,

    /// Override input format detection for the trace.
    #[arg(long, value_name = "FORMAT", value_parser = parse_format)]
    pub from: Option<Format>,

    /// Write the PNG to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

/// A `min_lon,min_lat,max_lon,max_lat` box from the command line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BboxArg([f64; 4]);

impl BboxArg {
    pub fn into_inner(self) -> [f64; 4] {
        self.0
    }
}

/// Parse a bounding box in the order every map service's `bbox=` uses.
///
/// Longitude first, which is the opposite of how the `bounds` line reads. The
/// two orders are easy to confuse and a transposed box downloads a map from the
/// other side of the world, so both numbers are range-checked against their own
/// axis — a latitude in a longitude slot is usually still in range, but a
/// longitude in a latitude slot is caught here rather than in a puzzling image.
fn parse_bbox(value: &str) -> Result<BboxArg, String> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    let [min_lon, min_lat, max_lon, max_lat] = parts.as_slice() else {
        return Err(format!(
            "expected MIN_LON,MIN_LAT,MAX_LON,MAX_LAT (four comma-separated \
             numbers), got `{value}`"
        ));
    };

    let number = |text: &str, name: &str| -> Result<f64, String> {
        text.parse::<f64>()
            .map_err(|_| format!("`{text}` is not a number for {name}"))
    };

    let (min_lon, min_lat, max_lon, max_lat) = (
        number(min_lon, "minimum longitude")?,
        number(min_lat, "minimum latitude")?,
        number(max_lon, "maximum longitude")?,
        number(max_lat, "maximum latitude")?,
    );

    for (name, lat) in [("minimum", min_lat), ("maximum", max_lat)] {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(format!("{name} latitude {lat} is outside -90..=90"));
        }
    }
    for (name, lon) in [("minimum", min_lon), ("maximum", max_lon)] {
        if !(-180.0..=180.0).contains(&lon) {
            return Err(format!("{name} longitude {lon} is outside -180..=180"));
        }
    }
    // A box with no extent, or an inverted one, describes no image. Accepting
    // it would mean rendering onto coordinates that are infinities.
    if min_lon >= max_lon || min_lat >= max_lat {
        return Err(format!(
            "the box has no extent: expected MIN_LON,MIN_LAT,MAX_LON,MAX_LAT \
             with both minimums below their maximums, got `{value}`"
        ));
    }

    Ok(BboxArg([min_lon, min_lat, max_lon, max_lat]))
}

/// Parse `#RRGGBB`, with or without the hash.
fn parse_hex_color(value: &str) -> Result<[u8; 3], String> {
    let digits = value.strip_prefix('#').unwrap_or(value);
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("expected a colour as #RRGGBB, got `{value}`"));
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16);
    match (channel(0), channel(2), channel(4)) {
        (Ok(r), Ok(g), Ok(b)) => Ok([r, g, b]),
        _ => Err(format!("expected a colour as #RRGGBB, got `{value}`")),
    }
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
    use pathify_core::spatial::{DEFAULT_NOISE_THRESHOLD_M, MAX_PLAUSIBLE_SPEED_MPS};
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

    fn clean_args(argv: &[&str]) -> CleanArgs {
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Clean(args) => args,
            other => panic!("expected `clean`, parsed {other:?}"),
        }
    }

    fn merge_args(argv: &[&str]) -> MergeArgs {
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Merge(args) => args,
            other => panic!("expected `merge`, parsed {other:?}"),
        }
    }

    fn takeout_args(argv: &[&str]) -> TakeoutArgs {
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Takeout(args) => args,
            other => panic!("expected `takeout`, parsed {other:?}"),
        }
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// The defaults are spelled with their units for `--help`, so they have to
    /// still mean the constants they stand for.
    #[test]
    fn the_spelled_defaults_match_the_constants() {
        assert_eq!(
            parse_distance(DEFAULT_ELEVATION_THRESHOLD).unwrap(),
            DEFAULT_NOISE_THRESHOLD_M
        );
        assert_eq!(
            parse_speed(DEFAULT_MAX_SPEED).unwrap(),
            MAX_PLAUSIBLE_SPEED_MPS
        );
        assert_eq!(parse_duration(DEFAULT_SEGMENT_GAP).unwrap(), 120.0);

        let defaults = info_args(&["pathify", "info"]);
        assert_eq!(defaults.elevation_threshold, DEFAULT_NOISE_THRESHOLD_M);
        assert_eq!(
            clean_args(&["pathify", "clean"]).max_speed,
            MAX_PLAUSIBLE_SPEED_MPS
        );
        assert_eq!(
            merge_args(&["pathify", "merge", "a.gpx", "b.gpx"]).segment_gap,
            120.0
        );
        assert_eq!(
            takeout_args(&["pathify", "takeout", "t.zip"]).segment_gap,
            120.0
        );
    }

    /// Every measured flag takes its unit inline and hands the program the
    /// metric base unit, whichever unit was typed.
    #[test]
    fn measured_flags_accept_their_units_inline() {
        let info = info_args(&["pathify", "info", "r.gpx", "--elevation-threshold", "10ft"]);
        assert!((info.elevation_threshold - 3.048).abs() < 1e-9);

        let clean = clean_args(&[
            "pathify",
            "clean",
            "r.gpx",
            "--trim-ends",
            "200ft",
            "--max-speed",
            "216km/h",
        ]);
        assert!((clean.trim_ends.unwrap() - 60.96).abs() < 1e-9);
        assert!((clean.max_speed - 60.0).abs() < 1e-9);

        let merge = merge_args(&[
            "pathify",
            "merge",
            "a.gpx",
            "b.gpx",
            "--dedup-window",
            "2min",
            "--dedup-radius",
            "50ft",
            "--segment-gap",
            "1h",
        ]);
        assert_eq!(merge.dedup_window, Some(120.0));
        assert!((merge.dedup_radius.unwrap() - 15.24).abs() < 1e-9);
        assert_eq!(merge.segment_gap, 3600.0);

        let takeout = takeout_args(&["pathify", "takeout", "t.zip", "--segment-gap", "5min"]);
        assert_eq!(takeout.segment_gap, 300.0);
    }

    /// A fence radius is a distance like any other, so it takes a unit too —
    /// and the latitude keeps its minus sign either way.
    #[test]
    fn a_fence_radius_takes_a_unit() {
        let args = clean_args(&[
            "pathify",
            "clean",
            "r.gpx",
            "--redact-around",
            "-33.8688,151.2093,500ft",
        ]);
        assert_eq!(args.redact_around[0].lat, -33.8688);
        assert!((args.redact_around[0].radius_m - 152.4).abs() < 1e-9);
    }

    /// A number with no unit keeps meaning the metric base unit, which is what
    /// every existing script and README example relies on.
    #[test]
    fn a_bare_number_still_means_meters_or_seconds() {
        let args = clean_args(&["pathify", "clean", "r.gpx", "--trim-ends", "300"]);
        assert_eq!(args.trim_ends, Some(300.0));
        let args = merge_args(&["pathify", "merge", "a.gpx", "b.gpx", "--dedup-window", "15"]);
        assert_eq!(args.dedup_window, Some(15.0));
    }

    #[test]
    fn a_unit_from_the_wrong_dimension_is_rejected() {
        // A duration is not a distance, however sensible the unit looks.
        assert!(Cli::try_parse_from(["pathify", "clean", "r.gpx", "--trim-ends", "5min"]).is_err());
        // And a speed is not a bare distance-over-nothing.
        assert!(Cli::try_parse_from(["pathify", "clean", "r.gpx", "--max-speed", "60ft"]).is_err());
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

    /// A Zip has no format to inherit, so `takeout` guesses where `convert`
    /// refuses to — but only after both of the ways the user could have said.
    #[test]
    fn takeout_falls_back_to_gpx_only_when_nothing_says_otherwise() {
        assert_eq!(
            takeout_args(&["pathify", "takeout", "t.zip"]).target_format(),
            Format::Gpx
        );
        assert_eq!(
            takeout_args(&["pathify", "takeout", "t.zip", "-o", "out/walks.geojson"])
                .target_format(),
            Format::GeoJson
        );
        assert_eq!(
            takeout_args(&[
                "pathify",
                "takeout",
                "t.zip",
                "--to",
                "csv",
                "-o",
                "walks.gpx"
            ])
            .target_format(),
            Format::Csv
        );
    }

    /// `--per-activity` writes a directory of files, so the output path is a
    /// directory name, and a directory name implies no format the way
    /// `walks.csv` does.
    #[test]
    fn a_per_activity_output_directory_does_not_choose_the_format() {
        let args = takeout_args(&[
            "pathify",
            "takeout",
            "t.zip",
            "--per-activity",
            "-o",
            "walks.csv",
        ]);
        assert!(args.per_activity);
        assert_eq!(args.target_format(), Format::Gpx);

        let args = takeout_args(&[
            "pathify",
            "takeout",
            "t.zip",
            "--per-activity",
            "--to",
            "csv",
            "-o",
            "walks",
        ]);
        assert_eq!(args.target_format(), Format::Csv);
    }

    /// The two are enforced by clap rather than checked at the point of use:
    /// there is nowhere for a file per activity to go without a directory,
    /// and nothing to split up when `--list` is only printing a table.
    #[test]
    fn per_activity_needs_an_output_directory_and_is_not_a_listing() {
        assert!(Cli::try_parse_from(["pathify", "takeout", "t.zip", "--per-activity"]).is_err());
        assert!(
            Cli::try_parse_from([
                "pathify",
                "takeout",
                "t.zip",
                "--per-activity",
                "--list",
                "-o",
                "walks"
            ])
            .is_err()
        );
    }

    /// `--json` is `--list`'s output format, not the command's: everything
    /// else `takeout` produces is a trace, which has `--to` for the purpose.
    #[test]
    fn takeout_json_belongs_to_the_listing() {
        let args = takeout_args(&["pathify", "takeout", "t.zip", "--list", "--json"]);
        assert!(args.list && args.json);
        assert!(Cli::try_parse_from(["pathify", "takeout", "t.zip", "--json"]).is_err());
        assert!(
            Cli::try_parse_from([
                "pathify", "takeout", "t.zip", "--list", "--json", "--type", "walk"
            ])
            .is_err()
        );
    }

    /// The type filter is comma-separated and space-tolerant, and `--list`
    /// cannot be combined with it: one prints a table, the other emits a
    /// trace.
    #[test]
    fn takeout_takes_a_comma_separated_type_filter() {
        let args = takeout_args(&["pathify", "takeout", "t.zip", "--type", "walk,outdoor bike"]);
        assert_eq!(
            args.r#type.as_deref(),
            Some(["walk".to_string(), "outdoor bike".to_string()].as_slice())
        );
        assert!(!args.list);
        assert!(
            Cli::try_parse_from(["pathify", "takeout", "t.zip", "--list", "--type", "walk"])
                .is_err()
        );
    }

    /// A multi-part export is several files, and they have to be readable as
    /// one: the exercise logs and the GPS days can land in different parts.
    #[test]
    fn takeout_accepts_several_archives_and_needs_at_least_one() {
        let args = takeout_args(&["pathify", "takeout", "a-001.zip", "a-002.zip"]);
        assert_eq!(args.archives.len(), 2);
        assert!(Cli::try_parse_from(["pathify", "takeout"]).is_err());
    }

    #[test]
    fn convert_without_a_resolvable_target_says_how_to_fix_it() {
        let args = convert_args(&["pathify", "convert", "ride.gpx"]);
        let message = args.target_format().unwrap_err();
        assert!(message.contains("--to"), "{message}");
        assert!(message.contains("--output"), "{message}");
    }
}
