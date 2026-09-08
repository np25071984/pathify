use std::fs::File;
use std::io::{BufWriter, IsTerminal, Write};

use anyhow::{Context, Result, anyhow, bail};
use pathify_core::formats;
use pathify_core::{Trace, Units};
use pathify_render::{
    DEFAULT_FOG_OPACITY, DEFAULT_REVEAL_M, DEFAULT_TRACK_COLOR, DEFAULT_TRACK_WIDTH_PX, Mode,
    RenderOptions, Report,
};

use crate::cli::RenderArgs;
use crate::io::read_render_inputs;

/// Feet per meter, for a reveal radius given in the local unit.
const FEET_PER_METER: f64 = 3.280_839_895;
/// The imperial default reveal — a round number in feet, not a conversion of
/// the metric one, so `--help` can state it without a decimal tail.
const DEFAULT_REVEAL_FT: f64 = 20.0;

/// Aspect mismatch worth mentioning, as a fraction. A couple of percent is
/// rounding in whatever produced the image; a third is the wrong image.
const ASPECT_SKEW_TOLERANCE: f64 = 0.02;

pub fn run(args: &RenderArgs, out: &mut dyn Write) -> Result<()> {
    // Checked before anything is read, the way `view` checks for a terminal:
    // a PNG dumped into a terminal emulator is a screenful of garbage and
    // sometimes a wedged shell, and finding that out after the work is done
    // helps nobody.
    if args.output.is_none() && std::io::stdout().is_terminal() {
        bail!(
            "`render` writes a PNG, and stdout is a terminal.\n\
             Send it to a file with --output, or pipe it somewhere."
        );
    }

    let (trace_bytes, trace_path, basemap) =
        read_render_inputs(args.input.as_deref(), args.basemap.as_deref())?;
    let format = formats::detect(args.from, trace_path.as_deref(), &trace_bytes)?;
    let trace = formats::read(format, &trace_bytes)?;

    let options = RenderOptions {
        bbox: resolve_bbox(args, &trace)?,
        mode: resolve_mode(args)?,
        track_color: args.track_color.unwrap_or(DEFAULT_TRACK_COLOR),
        track_width_px: resolve_track_width(args)?,
    };

    let report = match &args.output {
        Some(destination) => {
            let file = File::create(destination)
                .with_context(|| format!("failed to create {}", destination.display()))?;
            let mut writer = BufWriter::new(file);
            let report = draw(&trace, &basemap, &options, &mut writer)?;
            writer
                .flush()
                .with_context(|| format!("failed to write {}", destination.display()))?;
            report
        }
        None => draw(&trace, &basemap, &options, out)?,
    };

    warn_about_aspect(&report);
    Ok(())
}

/// Render, translating the crate's errors into ones that classify correctly.
///
/// A malformed PNG reaches us as a `DecodingError` that may carry an
/// `io::Error` for a truncated file, and `exit_code` reads any `io::Error` in
/// the chain as an environment failure. But a basemap that will not decode is
/// bad input, so its cause is flattened into the message and it exits with the
/// data-error code. An encoding failure really is I/O — a closed pipe, a full
/// disk — so that one keeps its source and the I/O exit code with it.
fn draw(
    trace: &Trace,
    basemap: &[u8],
    options: &RenderOptions,
    out: &mut dyn Write,
) -> Result<Report> {
    pathify_render::render(trace, basemap, options, out).map_err(|error| match error {
        pathify_render::Error::Encode(_) => anyhow::Error::new(error),
        other => anyhow!("{other}"),
    })
}

/// The box the basemap covers.
///
/// Defaulting to the trace's own bounds is right for the intended workflow —
/// the image was downloaded for the bbox `pathify info` printed — and wrong if
/// it was not, which is what `--bbox` and the aspect warning are for.
fn resolve_bbox(args: &RenderArgs, trace: &Trace) -> Result<[f64; 4]> {
    if let Some(bbox) = args.bbox {
        return Ok(bbox.into_inner());
    }
    let bounds = trace
        .bounds()
        .ok_or_else(|| anyhow!("the trace has no points, so there is nothing to draw"))?;

    let bbox = bounds.bbox();
    // A trace that never moved has bounds with no extent, so there is no box
    // to infer. The basemap does cover an area, and only the caller knows it.
    if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        bail!(
            "the trace has no extent, so the basemap's area cannot be inferred \
             from it.\nPass --bbox with the box the image was downloaded for."
        );
    }
    Ok(bbox)
}

fn resolve_mode(args: &RenderArgs) -> Result<Mode> {
    if !args.fog {
        return Ok(Mode::Plain);
    }

    // `--reveal` arrives already in meters: it carries its unit inline like
    // every other measured flag, and a bare number means meters wherever the
    // host is. The locale only chooses the default, so a reader who thinks in
    // feet gets a round number of them when they say nothing at all.
    let reveal_m = match args.reveal {
        Some(given) if given > 0.0 => given,
        // `parse_distance` has already refused a negative or non-finite
        // radius; zero is the one remaining value it allows and fog cannot
        // use, since it would clear nothing and hide the whole map.
        Some(_) => bail!("--reveal must be a distance greater than zero"),
        None => match Units::detect() {
            Units::Metric => DEFAULT_REVEAL_M,
            Units::Imperial => DEFAULT_REVEAL_FT / FEET_PER_METER,
        },
    };

    let opacity = args.fog_opacity.unwrap_or(DEFAULT_FOG_OPACITY);
    if !(0.0..=1.0).contains(&opacity) {
        bail!("--fog-opacity {opacity} must be between 0 and 1");
    }

    Ok(Mode::Fog { reveal_m, opacity })
}

fn resolve_track_width(args: &RenderArgs) -> Result<f64> {
    match args.track_width {
        Some(width) if width > 0.0 && width.is_finite() => Ok(width),
        Some(width) => bail!("--track-width {width} must be greater than zero"),
        None => Ok(DEFAULT_TRACK_WIDTH_PX),
    }
}

/// Say so when the image is not the shape its bounding box implies.
///
/// A warning rather than a failure: the picture is still useful, and the aspect
/// ratio is only evidence. But it is good evidence — the usual cause is a
/// basemap downloaded for a different area than the `--bbox` passed with it,
/// and the resulting image looks entirely plausible while putting the route in
/// the wrong place.
fn warn_about_aspect(report: &Report) {
    if (report.aspect_skew - 1.0).abs() <= ASPECT_SKEW_TOLERANCE {
        return;
    }

    let expected_width = (f64::from(report.width) / report.aspect_skew).round();
    eprintln!(
        "pathify: the basemap is {}x{} px, but its bounding box is the shape of \
         {}x{} — the trace will be stretched.\n\
         Check that --bbox matches the area the image was downloaded for.",
        report.width, report.height, expected_width as i64, report.height
    );
}
