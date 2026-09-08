//! An RGBA canvas, and the coverage masks that get painted onto it.
//!
//! Nothing in Pathify drew a line wider than a pixel before this: braille dots
//! have no thickness to configure. So stroking is built here from a distance
//! field rather than from Bresenham's algorithm — for the same amount of code
//! it gives antialiased edges and round joins, and it is the same computation
//! the fog-of-war corridor needs, so the two modes share it.

use std::io::Write;

use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};

use crate::{Error, Result};

/// Antialiasing feather for a stroke edge, in pixels. One pixel of ramp is
/// enough to take the staircase off a diagonal without looking blurred.
const STROKE_FEATHER_PX: f64 = 1.0;

/// Per-pixel coverage in 0..=1, the size of the image it will be painted onto.
///
/// Built up before anything is painted, and combined by taking the maximum, so
/// a route that crosses itself or doubles back does not darken twice where it
/// overlaps.
pub struct Mask {
    values: Vec<f32>,
    width: u32,
    height: u32,
}

impl Mask {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            values: vec![0.0; (width as usize) * (height as usize)],
            width,
            height,
        }
    }

    /// Whether any pixel is covered at all.
    ///
    /// The honest test for "did this trace land on this map": a trace whose
    /// every point is off-frame still counts if a segment crosses the frame,
    /// and no cheaper check gets that right.
    pub fn is_blank(&self) -> bool {
        self.values.iter().all(|v| *v <= 0.0)
    }

    pub fn get(&self, x: u32, y: u32) -> f32 {
        self.values[(y as usize) * (self.width as usize) + (x as usize)]
    }

    /// Cover everything within `inner` pixels of the polyline, fading to
    /// nothing over a further `feather` pixels.
    ///
    /// The polyline is one segment of the trace. Callers must not concatenate
    /// segments into a single one: a segment break is a pause or a dropout, and
    /// a line across it draws travel that never happened.
    pub fn add_polyline(&mut self, points: &[(f64, f64)], inner: f64, feather: f64) {
        match points {
            [] => {}
            // A one-point segment has no line to stroke, so it becomes a dot.
            // Dropping it would let a trace disappear entirely.
            [only] => self.add_capsule(*only, *only, inner, feather),
            _ => {
                for pair in points.windows(2) {
                    self.add_capsule(pair[0], pair[1], inner, feather);
                }
            }
        }
    }

    /// Cover a segment thickened into a round-capped capsule.
    fn add_capsule(&mut self, a: (f64, f64), b: (f64, f64), inner: f64, feather: f64) {
        let feather = feather.max(f64::EPSILON);
        let reach = inner + feather;

        walk_capsule_pixels(self.width, self.height, a, b, reach, |x, y, distance| {
            let coverage = (((reach - distance) / feather) as f32).clamp(0.0, 1.0);
            let slot = &mut self.values[(y as usize) * (self.width as usize) + (x as usize)];
            // Maximum rather than sum: overlapping strokes should not stack
            // into a darker patch where a route crosses itself.
            *slot = slot.max(coverage);
        });
    }
}

/// An 8-bit RGBA image, decoded from a PNG and encoded back to one.
pub struct Canvas {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    /// Whether the source PNG had an alpha channel. Kept so the output has one
    /// exactly when the input did — inventing a channel bloats the file, and
    /// dropping one silently flattens a transparent map onto black.
    has_alpha: bool,
}

impl Canvas {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Decode a PNG into RGBA8.
    ///
    /// `normalize_to_color8` handles the awkward inputs — a palette, 16 bits
    /// per channel, a `tRNS` chunk — so the rest of this module only ever sees
    /// four 8-bit channels, whatever was actually downloaded.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(bytes);
        decoder.set_transformations(Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(Error::Decode)?;

        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).map_err(Error::Decode)?;
        let (width, height) = (frame.width, frame.height);
        let source = &buffer[..frame.buffer_size()];

        let (channels, has_alpha) = match frame.color_type {
            ColorType::Grayscale => (1, false),
            ColorType::GrayscaleAlpha => (2, true),
            ColorType::Rgb => (3, false),
            ColorType::Rgba => (4, true),
            other => return Err(Error::UnsupportedColorType(other)),
        };

        let count = (width as usize) * (height as usize);
        let mut pixels = Vec::with_capacity(count * 4);
        for pixel in source.chunks_exact(channels) {
            let (rgb, alpha) = match pixel {
                [gray] => ([*gray, *gray, *gray], 255),
                [gray, alpha] => ([*gray, *gray, *gray], *alpha),
                [r, g, b] => ([*r, *g, *b], 255),
                [r, g, b, alpha] => ([*r, *g, *b], *alpha),
                _ => unreachable!("chunks_exact yields exactly `channels` bytes"),
            };
            pixels.extend_from_slice(&rgb);
            pixels.push(alpha);
        }

        Ok(Self {
            pixels,
            width,
            height,
            has_alpha,
        })
    }

    pub fn encode(&self, out: &mut dyn Write) -> Result<()> {
        let mut encoder = Encoder::new(out, self.width, self.height);
        encoder.set_depth(BitDepth::Eight);

        let data: Vec<u8> = if self.has_alpha {
            encoder.set_color(ColorType::Rgba);
            self.pixels.clone()
        } else {
            encoder.set_color(ColorType::Rgb);
            self.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        };

        let mut writer = encoder.write_header().map_err(Error::Encode)?;
        writer.write_image_data(&data).map_err(Error::Encode)?;
        writer.finish().map_err(Error::Encode)
    }

    /// Paint `color` over the canvas wherever `mask` covers it.
    ///
    /// Straight source-over compositing, alpha included, so a stroke across a
    /// transparent corner of a basemap comes out opaque rather than ghostly.
    pub fn blend(&mut self, mask: &Mask, color: [u8; 3]) {
        for y in 0..self.height {
            for x in 0..self.width {
                let source_alpha = f64::from(mask.get(x, y));
                if source_alpha <= 0.0 {
                    continue;
                }

                let slot = self.pixel_mut(x, y);
                let dest_alpha = f64::from(slot[3]) / 255.0;
                let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
                for channel in 0..3 {
                    let source = f64::from(color[channel]);
                    let dest = f64::from(slot[channel]);
                    slot[channel] = if out_alpha > 0.0 {
                        let mixed = (source * source_alpha
                            + dest * dest_alpha * (1.0 - source_alpha))
                            / out_alpha;
                        mixed.round().clamp(0.0, 255.0) as u8
                    } else {
                        0
                    };
                }
                slot[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    /// Darken the canvas by `opacity`, held back by `revealed`.
    ///
    /// Where the mask is 1 the pixel is untouched; where it is 0 the pixel is
    /// scaled by `1 - opacity`. Alpha is left alone: fog dims the map, it does
    /// not make it see-through.
    pub fn darken(&mut self, revealed: &Mask, opacity: f64) {
        let opacity = opacity.clamp(0.0, 1.0);
        for y in 0..self.height {
            for x in 0..self.width {
                let factor = 1.0 - opacity * (1.0 - f64::from(revealed.get(x, y)));
                let slot = self.pixel_mut(x, y);
                // Alpha, the fourth byte, is deliberately left out: fog dims
                // the map, it does not make it see-through.
                for channel in slot.iter_mut().take(3) {
                    *channel = (f64::from(*channel) * factor).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    fn pixel_mut(&mut self, x: u32, y: u32) -> &mut [u8] {
        let start = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        &mut self.pixels[start..start + 4]
    }

    #[cfg(test)]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let start = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.pixels[start..start + 4].try_into().unwrap()
    }
}

/// Coverage feather and inner radius for a stroke of the given width.
pub fn stroke_shape(width_px: f64) -> (f64, f64) {
    // Half the width, less half the feather, so the ramp straddles the nominal
    // edge instead of sitting entirely inside or outside it.
    let inner = (width_px / 2.0 - STROKE_FEATHER_PX / 2.0).max(0.0);
    (inner, STROKE_FEATHER_PX)
}

/// Visit every pixel within `reach` of the segment `a`–`b`, with its distance.
///
/// The segment is walked in short chunks rather than covered by one bounding
/// box. A box around a segment that crosses the whole image is the whole image,
/// and a dense trace would then cost the image area per segment; chunking keeps
/// the work proportional to the line's length instead. Chunks overlap, so a
/// pixel can be visited more than once — which is why coverage is combined by
/// maximum and never by addition.
fn walk_capsule_pixels(
    width: u32,
    height: u32,
    a: (f64, f64),
    b: (f64, f64),
    reach: f64,
    mut visit: impl FnMut(u32, u32, f64),
) {
    if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
        return;
    }

    let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let chunk_length = (reach * 2.0).max(1.0);
    let chunks = (length / chunk_length).ceil().max(1.0);
    // A chunk count beyond the pixel count cannot buy any accuracy and would
    // only spin: clamp it to something an image can distinguish.
    let chunks = chunks.min(f64::from(width) + f64::from(height) + 2.0) as u32;

    for chunk in 0..chunks {
        let t0 = f64::from(chunk) / f64::from(chunks);
        let t1 = f64::from(chunk + 1) / f64::from(chunks);
        let from = (a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0);
        let to = (a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1);

        let min_x = (from.0.min(to.0) - reach).floor().max(0.0) as u32;
        let min_y = (from.1.min(to.1) - reach).floor().max(0.0) as u32;
        let max_x = ((from.0.max(to.0) + reach).ceil()).min(f64::from(width) - 1.0);
        let max_y = ((from.1.max(to.1) + reach).ceil()).min(f64::from(height) - 1.0);
        if max_x < 0.0 || max_y < 0.0 {
            continue;
        }

        for y in min_y..=(max_y as u32) {
            for x in min_x..=(max_x as u32) {
                // Pixel centres, not corners, or the stroke sits half a pixel
                // up and to the left of where it belongs.
                let distance =
                    distance_to_segment(f64::from(x) + 0.5, f64::from(y) + 0.5, from, to);
                if distance <= reach {
                    visit(x, y, distance);
                }
            }
        }
    }
}

/// Shortest distance from a point to a line segment. Handles a zero-length
/// segment, which is how a single-point trace is drawn.
fn distance_to_segment(px: f64, py: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > 0.0 {
        (((px - a.0) * dx + (py - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (nx, ny) = (a.0 + dx * t, a.1 + dy * t);
    ((px - nx).powi(2) + (py - ny).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_segment_uses_the_nearest_point_on_it() {
        let (a, b) = ((0.0, 0.0), (10.0, 0.0));
        // Alongside: the perpendicular distance.
        assert!((distance_to_segment(5.0, 3.0, a, b) - 3.0).abs() < 1e-12);
        // Past the end: the distance to the endpoint, not to the infinite line.
        assert!((distance_to_segment(14.0, 0.0, a, b) - 4.0).abs() < 1e-12);
        assert!((distance_to_segment(-3.0, 4.0, a, b) - 5.0).abs() < 1e-12);
    }

    /// A zero-length segment is a point, not a division by zero.
    #[test]
    fn a_degenerate_segment_measures_to_its_own_position() {
        let p = (4.0, 4.0);
        assert!((distance_to_segment(4.0, 7.0, p, p) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn a_stroke_covers_its_own_width_and_stops() {
        let mut mask = Mask::new(40, 40);
        let (inner, feather) = stroke_shape(4.0);
        mask.add_polyline(&[(0.0, 20.5), (39.0, 20.5)], inner, feather);

        // Dead centre of a 4 px stroke is fully covered.
        assert_eq!(mask.get(20, 20), 1.0);
        assert_eq!(mask.get(20, 21), 1.0);
        // Four pixels out from the centre line is well clear of it.
        assert_eq!(mask.get(20, 16), 0.0);
        assert_eq!(mask.get(20, 25), 0.0);
    }

    /// The chunked traversal must not leave gaps along a diagonal — that would
    /// show up as a dashed line, which reads as a series of dropouts.
    #[test]
    fn a_long_diagonal_stroke_has_no_holes() {
        let mut mask = Mask::new(64, 64);
        let (inner, feather) = stroke_shape(2.0);
        mask.add_polyline(&[(0.5, 0.5), (63.5, 63.5)], inner, feather);

        for i in 0..64 {
            assert!(
                mask.get(i, i) > 0.5,
                "the diagonal is missing at pixel {i}: {}",
                mask.get(i, i)
            );
        }
    }

    /// A point far off the image must not be walked pixel by pixel all the way
    /// in; it must also not be dropped, since the line it belongs to may still
    /// cross the frame.
    #[test]
    fn a_line_from_far_off_frame_still_crosses_the_image() {
        let mut mask = Mask::new(32, 32);
        let (inner, feather) = stroke_shape(3.0);
        mask.add_polyline(&[(-100_000.0, 16.0), (100_000.0, 16.0)], inner, feather);

        assert!(mask.get(0, 16) > 0.0);
        assert!(mask.get(31, 16) > 0.0);
        assert_eq!(mask.get(16, 0), 0.0);
    }

    #[test]
    fn a_segment_entirely_off_frame_covers_nothing() {
        let mut mask = Mask::new(32, 32);
        let (inner, feather) = stroke_shape(3.0);
        mask.add_polyline(&[(500.0, 500.0), (600.0, 600.0)], inner, feather);
        assert!(mask.is_blank());
    }

    /// Overlaps take the maximum. Summing them would leave a darker smudge
    /// wherever a route crossed itself.
    #[test]
    fn overlapping_strokes_do_not_stack() {
        let mut mask = Mask::new(20, 20);
        let (inner, feather) = stroke_shape(6.0);
        mask.add_polyline(&[(0.0, 10.0), (19.0, 10.0)], inner, feather);
        mask.add_polyline(&[(10.0, 0.0), (10.0, 19.0)], inner, feather);
        assert_eq!(mask.get(10, 10), 1.0);
    }

    fn solid_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, width, height);
        encoder.set_color(ColorType::Rgb);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let row: Vec<u8> = (0..width).flat_map(|_| rgb).collect();
        let data: Vec<u8> = (0..height).flat_map(|_| row.clone()).collect();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn a_png_survives_a_decode_and_encode_round_trip() {
        let original = solid_png(8, 4, [10, 20, 30]);
        let canvas = Canvas::decode(&original).unwrap();
        assert_eq!((canvas.width(), canvas.height()), (8, 4));
        assert_eq!(canvas.pixel(3, 2), [10, 20, 30, 255]);
        assert!(!canvas.has_alpha);

        let mut encoded = Vec::new();
        canvas.encode(&mut encoded).unwrap();
        let again = Canvas::decode(&encoded).unwrap();
        assert_eq!(again.pixel(3, 2), [10, 20, 30, 255]);
    }

    #[test]
    fn blending_replaces_fully_covered_pixels_and_leaves_the_rest() {
        let mut canvas = Canvas::decode(&solid_png(8, 8, [200, 200, 200])).unwrap();
        let mut mask = Mask::new(8, 8);
        let (inner, feather) = stroke_shape(2.0);
        mask.add_polyline(&[(4.5, 0.0), (4.5, 7.0)], inner, feather);

        canvas.blend(&mask, [255, 0, 0]);
        assert_eq!(canvas.pixel(4, 4), [255, 0, 0, 255]);
        assert_eq!(canvas.pixel(0, 0), [200, 200, 200, 255]);
    }

    #[test]
    fn darkening_scales_uncovered_pixels_by_one_minus_opacity() {
        let mut canvas = Canvas::decode(&solid_png(4, 4, [200, 100, 50])).unwrap();
        let revealed = Mask::new(4, 4);
        canvas.darken(&revealed, 0.75);
        assert_eq!(canvas.pixel(2, 2), [50, 25, 13, 255]);
    }
}
