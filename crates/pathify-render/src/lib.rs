//! Draw a Pathify trace onto a bitmap basemap.
//!
//! Pathify has no network code and is not going to get any — the guarantee
//! that a location history never leaves the machine is only credible if the
//! program is *incapable* of a request. That rules out fetching map tiles. It
//! does not rule out drawing on a map the user already has, so this crate takes
//! a PNG someone downloaded themselves, plus the geographic box it covers, and
//! composites a trace onto it.
//!
//! Kept as its own crate for the same reason `pathify-tui` is: so that `png`
//! never enters the dependency graph of the pipeline commands, and so the
//! projection and rasterizing stay testable without a CLI.
//!
//! Two modes, both driven from the same distance-field code:
//!
//! - [`Mode::Plain`] strokes the track over the map, with start and end marks.
//! - [`Mode::Fog`] darkens the whole map and reveals a clear corridor of real
//!   map along the route — the ground actually covered, and nothing else.

use std::io::Write;

use pathify_core::Trace;

pub mod mercator;
pub mod raster;

pub use mercator::Basemap;

/// The 8-byte signature every PNG file starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Cyan, matching the terminal map's `TRACK_COLOR`. Bright enough to read over
/// both the pale fills and the dark labels a street map is made of.
pub const DEFAULT_TRACK_COLOR: [u8; 3] = [0, 174, 239];
/// Green start and red end, the terminal map's convention in concrete RGB.
pub const START_COLOR: [u8; 3] = [45, 200, 90];
pub const END_COLOR: [u8; 3] = [230, 60, 60];

pub const DEFAULT_TRACK_WIDTH_PX: f64 = 3.0;
pub const DEFAULT_FOG_OPACITY: f64 = 0.65;
/// How far either side of the route the fog is lifted, by default.
pub const DEFAULT_REVEAL_M: f64 = 6.0;

/// Endpoint marker radius as a multiple of the stroke width, so a thicker
/// track gets proportionally larger marks instead of losing them.
const ENDPOINT_RADIUS_FACTOR: f64 = 1.6;
/// The reveal edge fades over this fraction of the reveal radius. Without a
/// ramp the corridor looks like a cut-out rather than clearing weather.
const REVEAL_FEATHER_FRACTION: f64 = 0.25;

#[derive(Debug)]
pub enum Error {
    Decode(png::DecodingError),
    Encode(png::EncodingError),
    UnsupportedColorType(png::ColorType),
    /// The basemap has no pixels.
    EmptyImage,
    /// The bounding box has no extent on one or both axes.
    DegenerateBbox,
    /// Nothing was drawn: no part of the trace falls inside the basemap.
    TraceOutsideBasemap,
    /// The trace has no points to draw.
    NoPoints,
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Decode(e) => write!(f, "the basemap is not a readable PNG: {e}"),
            Error::Encode(e) => write!(f, "failed to write the rendered PNG: {e}"),
            Error::UnsupportedColorType(c) => {
                write!(f, "the basemap uses an unsupported PNG color type: {c:?}")
            }
            Error::EmptyImage => write!(f, "the basemap has no pixels"),
            Error::DegenerateBbox => write!(
                f,
                "the bounding box has no extent: give --bbox a box whose \
                 longitudes and latitudes both differ"
            ),
            Error::TraceOutsideBasemap => write!(
                f,
                "no part of the trace falls inside the basemap's bounding box"
            ),
            Error::NoPoints => write!(f, "the trace has no points"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Decode(e) => Some(e),
            Error::Encode(e) => Some(e),
            _ => None,
        }
    }
}

/// Whether these bytes are a PNG.
///
/// Used by the CLI to tell a piped basemap from a piped trace, so that both
/// `cat map.png | pathify render ride.gpx` and
/// `cat ride.gpx | pathify render --basemap map.png` mean what they look like.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIGNATURE)
}

/// What to draw, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// The area the basemap covers, as `[min_lon, min_lat, max_lon, max_lat]`.
    pub bbox: [f64; 4],
    pub mode: Mode,
    pub track_color: [u8; 3],
    pub track_width_px: f64,
}

impl RenderOptions {
    /// Plain mode over the given box, with the default colour and width.
    pub fn new(bbox: [f64; 4]) -> Self {
        Self {
            bbox,
            mode: Mode::Plain,
            track_color: DEFAULT_TRACK_COLOR,
            track_width_px: DEFAULT_TRACK_WIDTH_PX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Stroke the track over the map as it is.
    Plain,
    /// Darken the map, and reveal it again along the route.
    ///
    /// No line is stroked in this mode: the clear corridor *is* the track, and
    /// a line down the middle of it would only repeat what the shape says.
    Fog {
        /// Radius of the cleared corridor, in ground meters.
        reveal_m: f64,
        /// How dark the fog is, from 0 (none) to 1 (black).
        opacity: f64,
    },
}

/// What the render turned out to be, for a caller that wants to report it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub meters_per_pixel: f64,
    /// See [`Basemap::aspect_skew`] — 1.0 means the image's proportions match
    /// the box it is said to cover.
    pub aspect_skew: f64,
}

/// Composite `trace` onto `basemap`, writing a PNG to `out`.
///
/// Nothing is written unless the whole render succeeds, so a failure cannot
/// leave a half-finished image behind on stdout.
pub fn render(
    trace: &Trace,
    basemap: &[u8],
    options: &RenderOptions,
    out: &mut dyn Write,
) -> Result<Report> {
    let mut canvas = raster::Canvas::decode(basemap)?;
    let map = Basemap::new(options.bbox, canvas.width(), canvas.height())?;

    // One polyline per segment, and they are never joined: a segment break is
    // where the recording paused or lost signal, and a stroke across it draws
    // travel nobody made.
    let segments: Vec<Vec<(f64, f64)>> = trace
        .segments()
        .map(|segment| {
            segment
                .points
                .iter()
                .map(|p| map.to_pixel(p.lat, p.lon))
                .collect()
        })
        .filter(|points: &Vec<_>| !points.is_empty())
        .collect();
    if segments.is_empty() {
        return Err(Error::NoPoints);
    }

    match options.mode {
        Mode::Plain => draw_plain(&mut canvas, &segments, options)?,
        Mode::Fog { reveal_m, opacity } => {
            let reveal_px = reveal_m / map.meters_per_pixel();
            draw_fog(&mut canvas, &segments, reveal_px, opacity)?;
        }
    }

    canvas.encode(out)?;
    Ok(Report {
        width: canvas.width(),
        height: canvas.height(),
        meters_per_pixel: map.meters_per_pixel(),
        aspect_skew: map.aspect_skew(),
    })
}

fn draw_plain(
    canvas: &mut raster::Canvas,
    segments: &[Vec<(f64, f64)>],
    options: &RenderOptions,
) -> Result<()> {
    let (width, height) = (canvas.width(), canvas.height());
    let (inner, feather) = raster::stroke_shape(options.track_width_px);

    // Every segment goes into one mask before anything is painted, so a route
    // that crosses itself blends once rather than compounding at the crossing.
    let mut track = raster::Mask::new(width, height);
    for points in segments {
        track.add_polyline(points, inner, feather);
    }
    if track.is_blank() {
        return Err(Error::TraceOutsideBasemap);
    }
    canvas.blend(&track, options.track_color);

    // Marks go on afterwards, so they sit on top of the line rather than being
    // swallowed by it where a route starts and finishes in the same place.
    let radius = options.track_width_px * ENDPOINT_RADIUS_FACTOR;
    let (start, end) = endpoints(segments);
    for (position, color) in [(start, START_COLOR), (end, END_COLOR)] {
        let mut mark = raster::Mask::new(width, height);
        mark.add_polyline(&[position], radius, feather);
        canvas.blend(&mark, color);
    }
    Ok(())
}

fn draw_fog(
    canvas: &mut raster::Canvas,
    segments: &[Vec<(f64, f64)>],
    reveal_px: f64,
    opacity: f64,
) -> Result<()> {
    let feather = (reveal_px * REVEAL_FEATHER_FRACTION).max(1.0);
    let mut revealed = raster::Mask::new(canvas.width(), canvas.height());
    for points in segments {
        revealed.add_polyline(points, reveal_px, feather);
    }
    if revealed.is_blank() {
        return Err(Error::TraceOutsideBasemap);
    }
    canvas.darken(&revealed, opacity);
    Ok(())
}

/// First and last drawn positions. Both exist: the caller has already rejected
/// a trace with no non-empty segment.
fn endpoints(segments: &[Vec<(f64, f64)>]) -> ((f64, f64), (f64, f64)) {
    let start = segments[0][0];
    let end = *segments
        .last()
        .and_then(|points| points.last())
        .unwrap_or(&start);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_png_by_its_signature() {
        assert!(is_png(&[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0
        ]));
        assert!(!is_png(b"<?xml version=\"1.0\"?><gpx"));
        assert!(!is_png(b"lat,lon\n47.6,-122.3\n"));
        // A truncated signature is not a PNG, and must not panic.
        assert!(!is_png(&[0x89, b'P']));
        assert!(!is_png(&[]));
    }
}
