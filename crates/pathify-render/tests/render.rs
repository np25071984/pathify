//! End-to-end renders, checked pixel by pixel.
//!
//! Everything here is built in memory — a synthetic basemap encoded with `png`,
//! rendered, and decoded again — the same way `pathify-tui` renders into a
//! `TestBackend` rather than a real terminal. A flat-coloured basemap makes
//! every assertion exact: any pixel that changed, changed because this crate
//! changed it.

use pathify_core::{Metadata, Point, Segment, Trace, Track};
use pathify_render::{Error, Mode, RenderOptions, render};

/// The box every test in this file renders into, and its matching image shape.
/// One degree square, centred on the equator and the prime meridian: at that
/// latitude Mercator is barely distorted, so pixel positions are easy to reason
/// about by hand.
///
/// It is also 111 km across at 557 m per pixel, so the fog reveal radii below
/// are in kilometres. The default 200 m would be a third of a pixel here —
/// which is the correct behaviour, just not a corridor anyone can assert on.
const BBOX: [f64; 4] = [-0.5, -0.5, 0.5, 0.5];
const SIZE: u32 = 200;

const BASE_RGB: [u8; 3] = [200, 200, 200];

fn basemap() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, SIZE, SIZE);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| BASE_RGB).collect();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
    bytes
}

/// Build a trace from one polyline per segment, each `(lat, lon)`.
fn trace(segments: &[&[(f64, f64)]]) -> Trace {
    Trace::new(
        Metadata::default(),
        vec![Track {
            name: None,
            description: None,
            segments: segments
                .iter()
                .map(|points| {
                    Segment::new(
                        points
                            .iter()
                            .map(|(lat, lon)| Point::new(*lat, *lon).unwrap())
                            .collect(),
                    )
                })
                .collect(),
        }],
    )
}

/// A horizontal line across the middle of the box.
fn across_the_middle() -> Trace {
    trace(&[&[(0.0, -0.4), (0.0, 0.4)]])
}

struct Rendered {
    pixels: Vec<u8>,
    width: u32,
}

impl Rendered {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let start = ((y as usize) * (self.width as usize) + (x as usize)) * 3;
        self.pixels[start..start + 3].try_into().unwrap()
    }

    fn is_untouched(&self, x: u32, y: u32) -> bool {
        self.at(x, y) == BASE_RGB
    }
}

fn render_to_pixels(trace: &Trace, options: &RenderOptions) -> Rendered {
    let mut out = Vec::new();
    render(trace, &basemap(), options, &mut out).expect("the render should succeed");

    assert!(
        pathify_render::is_png(&out),
        "the output should be a PNG, so it can be piped straight to a file"
    );

    let mut decoder = png::Decoder::new(out.as_slice());
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        frame.color_type,
        png::ColorType::Rgb,
        "an RGB basemap should not gain an alpha channel it never had"
    );
    pixels.truncate(frame.buffer_size());
    Rendered {
        pixels,
        width: frame.width,
    }
}

fn plain() -> RenderOptions {
    RenderOptions::new(BBOX)
}

fn fog(reveal_m: f64, opacity: f64) -> RenderOptions {
    RenderOptions {
        mode: Mode::Fog { reveal_m, opacity },
        ..RenderOptions::new(BBOX)
    }
}

#[test]
fn the_track_is_drawn_where_the_trace_went_and_nowhere_else() {
    let image = render_to_pixels(&across_the_middle(), &plain());

    // The line runs along the middle row, so the centre is the track colour.
    assert_eq!(
        image.at(SIZE / 2, SIZE / 2),
        pathify_render::DEFAULT_TRACK_COLOR
    );
    // A quarter of the image away from it, the basemap is untouched.
    assert!(image.is_untouched(SIZE / 2, SIZE / 4));
    assert!(image.is_untouched(SIZE / 2, SIZE * 3 / 4));
}

#[test]
fn the_ends_of_the_track_are_marked() {
    let image = render_to_pixels(&across_the_middle(), &plain());
    // The trace runs west to east, so the start is on the left.
    assert_eq!(image.at(20, SIZE / 2), pathify_render::START_COLOR);
    assert_eq!(image.at(180, SIZE / 2), pathify_render::END_COLOR);
}

/// Northward is up and eastward is right. This is the mistake that produces a
/// finished image that looks fine until you compare it with the streets.
#[test]
fn the_image_is_not_flipped_or_transposed() {
    let north_east = trace(&[&[(0.4, 0.3), (0.4, 0.35)]]);
    let image = render_to_pixels(&north_east, &plain());

    let mut found = None;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if !image.is_untouched(x, y) {
                found = Some((x, y));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    let (x, y) = found.expect("something should have been drawn");
    assert!(
        x > SIZE / 2,
        "a point east of centre belongs right of centre"
    );
    assert!(y < SIZE / 2, "a point north of centre belongs above centre");
}

/// The invariant stated throughout the codebase: a segment break is a pause or
/// a dropout, so the ground between two segments was never covered and must
/// not be drawn on.
#[test]
fn separate_segments_are_not_joined_by_a_line() {
    let gapped = trace(&[&[(0.0, -0.4), (0.0, -0.25)], &[(0.0, 0.25), (0.0, 0.4)]]);
    let image = render_to_pixels(&gapped, &plain());

    // Both segments drew.
    assert_eq!(image.at(20, SIZE / 2), pathify_render::START_COLOR);
    assert_eq!(image.at(180, SIZE / 2), pathify_render::END_COLOR);

    // The middle of the gap did not, on the row the join would have run along.
    assert!(
        image.is_untouched(SIZE / 2, SIZE / 2),
        "a stroke across the gap draws travel that never happened: {:?}",
        image.at(SIZE / 2, SIZE / 2)
    );
}

#[test]
fn fog_darkens_the_map_and_leaves_the_route_alone() {
    let image = render_to_pixels(&across_the_middle(), &fog(20_000.0, 0.75));

    // On the route, the basemap shows through untouched.
    assert!(
        image.is_untouched(SIZE / 2, SIZE / 2),
        "the corridor should be the original map: {:?}",
        image.at(SIZE / 2, SIZE / 2)
    );

    // Far from it, every channel is scaled by 1 - opacity.
    assert_eq!(image.at(SIZE / 2, 10), [50, 50, 50]);
}

/// The corridor is the whole point of fog mode: it has to be wider when asked
/// for a wider one, and in ground units rather than pixels.
#[test]
fn a_larger_reveal_clears_more_of_the_map() {
    let narrow = render_to_pixels(&across_the_middle(), &fog(5_000.0, 0.75));
    let wide = render_to_pixels(&across_the_middle(), &fog(30_000.0, 0.75));

    let cleared = |image: &Rendered| {
        (0..SIZE)
            .filter(|y| image.is_untouched(SIZE / 2, *y))
            .count()
    };
    assert!(
        cleared(&wide) > cleared(&narrow),
        "30 km should clear more than 5 km: {} vs {}",
        cleared(&wide),
        cleared(&narrow)
    );
}

#[test]
fn fog_does_not_reveal_a_corridor_across_a_segment_gap() {
    let gapped = trace(&[&[(0.0, -0.4), (0.0, -0.25)], &[(0.0, 0.25), (0.0, 0.4)]]);
    let image = render_to_pixels(&gapped, &fog(10_000.0, 0.75));

    assert!(
        image.is_untouched(20, SIZE / 2),
        "the first segment should be revealed"
    );
    assert_eq!(
        image.at(SIZE / 2, SIZE / 2),
        [50, 50, 50],
        "the gap between segments is ground nobody covered, so it stays fogged"
    );
}

/// A single fix is still somewhere the subject was, and a trace made of one is
/// still a trace. Silently rendering an unmarked map would be worse than
/// either drawing it or refusing.
#[test]
fn a_one_point_segment_still_draws() {
    let single = trace(&[&[(0.0, 0.0)]]);
    let image = render_to_pixels(&single, &plain());
    assert!(!image.is_untouched(SIZE / 2, SIZE / 2));
}

#[test]
fn a_trace_that_misses_the_basemap_is_an_error_rather_than_an_untouched_copy() {
    let elsewhere = trace(&[&[(51.5, -0.12), (51.51, -0.13)]]);
    let mut out = Vec::new();
    let error = render(&elsewhere, &basemap(), &plain(), &mut out).unwrap_err();
    assert!(matches!(error, Error::TraceOutsideBasemap), "{error:?}");
    assert!(
        out.is_empty(),
        "a failed render must not leave a half-written PNG behind"
    );
}

/// A line whose endpoints are both off the image still crosses it, and the
/// visible part has to be drawn.
#[test]
fn a_line_passing_through_the_frame_is_drawn_even_with_both_ends_outside() {
    let passing = trace(&[&[(0.0, -20.0), (0.0, 20.0)]]);
    let image = render_to_pixels(&passing, &plain());
    assert_eq!(
        image.at(SIZE / 2, SIZE / 2),
        pathify_render::DEFAULT_TRACK_COLOR
    );
    assert_eq!(image.at(0, SIZE / 2), pathify_render::DEFAULT_TRACK_COLOR);
}

#[test]
fn an_empty_trace_is_an_error() {
    let empty = Trace::new(Metadata::default(), vec![]);
    let mut out = Vec::new();
    let error = render(&empty, &basemap(), &plain(), &mut out).unwrap_err();
    assert!(matches!(error, Error::NoPoints), "{error:?}");
}

#[test]
fn a_box_with_no_extent_is_refused_with_advice() {
    let options = RenderOptions::new([0.0, 0.0, 0.0, 0.0]);
    let mut out = Vec::new();
    let error = render(&across_the_middle(), &basemap(), &options, &mut out).unwrap_err();
    assert!(matches!(error, Error::DegenerateBbox), "{error:?}");
    assert!(error.to_string().contains("--bbox"), "{error}");
}

#[test]
fn something_that_is_not_a_png_is_refused() {
    let mut out = Vec::new();
    let error = render(
        &across_the_middle(),
        b"not a png at all",
        &plain(),
        &mut out,
    )
    .unwrap_err();
    assert!(matches!(error, Error::Decode(_)), "{error:?}");
}

#[test]
fn the_report_describes_the_image_it_rendered() {
    let mut out = Vec::new();
    let report = render(&across_the_middle(), &basemap(), &plain(), &mut out).unwrap();
    assert_eq!((report.width, report.height), (SIZE, SIZE));
    // A one-degree box at the equator is about 111 km across 200 px.
    assert!(
        (report.meters_per_pixel - 556.6).abs() < 1.0,
        "{}",
        report.meters_per_pixel
    );
    // A square image for a square-in-Mercator box is not skewed.
    assert!(
        (report.aspect_skew - 1.0).abs() < 0.001,
        "{}",
        report.aspect_skew
    );
}

#[test]
fn the_track_colour_and_width_are_configurable() {
    let thick = RenderOptions {
        track_color: [255, 0, 255],
        track_width_px: 9.0,
        ..plain()
    };
    let thin = RenderOptions {
        track_width_px: 1.0,
        ..plain()
    };

    let thick_image = render_to_pixels(&across_the_middle(), &thick);
    assert_eq!(thick_image.at(SIZE / 2, SIZE / 2), [255, 0, 255]);

    let count = |image: &Rendered| {
        (0..SIZE)
            .filter(|y| !image.is_untouched(SIZE / 2, *y))
            .count()
    };
    assert!(
        count(&thick_image) > count(&render_to_pixels(&across_the_middle(), &thin)),
        "a 9 px stroke should cover more rows than a 1 px one"
    );
}
