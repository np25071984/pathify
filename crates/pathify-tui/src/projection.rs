//! Flattening latitude and longitude onto a drawable plane.
//!
//! There are no map tiles here and never will be — fetching them would mean
//! network access, which the whole tool is built to avoid. So this is a plain
//! plot of the trace itself, and the only thing the projection has to get right
//! is the shape: a degree of longitude is shorter than a degree of latitude
//! everywhere but the equator, and ignoring that draws every route stretched
//! sideways.
//!
//! An equirectangular projection anchored at the trace's own latitude is enough
//! for that. Over the extent of a single recording the error is far below one
//! terminal dot, and it keeps panning linear — a Mercator would stretch the
//! view as it scrolled north.

use pathify_core::Trace;

/// Cosine floor, so a trace at the pole cannot collapse the x axis to zero
/// width and produce infinities downstream.
const MIN_LON_SCALE: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    lon_scale: f64,
}

/// A rectangle in projected units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Projection {
    /// Anchor the projection at `reference_lat`, normally the middle of the
    /// trace being drawn.
    pub fn centered_on(reference_lat: f64) -> Self {
        Self {
            lon_scale: reference_lat.to_radians().cos().abs().max(MIN_LON_SCALE),
        }
    }

    /// Project to plane coordinates, in units of one degree of latitude.
    pub fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        (lon * self.lon_scale, lat)
    }

    /// Recover the latitude and longitude of a projected point.
    pub fn unproject(&self, x: f64, y: f64) -> (f64, f64) {
        (y, x / self.lon_scale)
    }
}

impl PlanarBounds {
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }
}

/// One polyline per segment, projected and ready to draw.
///
/// Segments stay separate all the way to the screen. Joining them would draw a
/// line across a pause or a signal loss, showing travel that never happened —
/// the same reason distance is never summed across a segment boundary.
pub struct Drawing {
    pub projection: Projection,
    pub segments: Vec<Vec<(f64, f64)>>,
    pub bounds: PlanarBounds,
}

impl Drawing {
    /// Project every segment of a trace, or `None` if it has no points.
    pub fn of(trace: &Trace) -> Option<Self> {
        let geographic = trace.bounds()?;
        let reference_lat = (geographic.min_lat + geographic.max_lat) / 2.0;
        let projection = Projection::centered_on(reference_lat);

        let segments: Vec<Vec<(f64, f64)>> = trace
            .segments()
            .map(|segment| {
                segment
                    .points
                    .iter()
                    .map(|p| projection.project(p.lat, p.lon))
                    .collect()
            })
            .filter(|points: &Vec<_>| !points.is_empty())
            .collect();

        if segments.is_empty() {
            return None;
        }

        let (min_x, _) = projection.project(0.0, geographic.min_lon);
        let (max_x, _) = projection.project(0.0, geographic.max_lon);
        Some(Self {
            projection,
            segments,
            bounds: PlanarBounds {
                min_x,
                min_y: geographic.min_lat,
                max_x,
                max_y: geographic.max_lat,
            },
        })
    }

    /// First and last drawn point, for marking where a journey began and ended.
    pub fn endpoints(&self) -> Option<((f64, f64), (f64, f64))> {
        let start = *self.segments.first()?.first()?;
        let end = *self.segments.last()?.last()?;
        Some((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pathify_core::{Metadata, Point, Segment, Track};

    fn trace_of(segments: Vec<Vec<(f64, f64)>>) -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: segments
                    .into_iter()
                    .map(|points| {
                        Segment::new(
                            points
                                .into_iter()
                                .map(|(lat, lon)| Point::new(lat, lon).unwrap())
                                .collect(),
                        )
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn latitude_maps_straight_through() {
        let projection = Projection::centered_on(47.6);
        let (_, y) = projection.project(47.6, -122.3);
        assert_eq!(y, 47.6);
    }

    /// The whole point of the projection: at 60° north a degree of longitude
    /// covers half the ground a degree of latitude does, and the drawing has to
    /// show that or every route comes out stretched sideways.
    #[test]
    fn longitude_shrinks_with_latitude() {
        let equator = Projection::centered_on(0.0);
        let sixty = Projection::centered_on(60.0);

        let (x_equator, _) = equator.project(0.0, 1.0);
        let (x_sixty, _) = sixty.project(60.0, 1.0);

        assert!((x_equator - 1.0).abs() < 1e-9);
        assert!((x_sixty - 0.5).abs() < 0.001, "got {x_sixty}");
    }

    #[test]
    fn projection_round_trips() {
        let projection = Projection::centered_on(47.6);
        let (x, y) = projection.project(47.61, -122.33);
        let (lat, lon) = projection.unproject(x, y);
        assert!((lat - 47.61).abs() < 1e-9);
        assert!((lon - -122.33).abs() < 1e-9);
    }

    /// At the pole the cosine is zero, which would collapse the x axis and
    /// produce infinities when the viewport divides by its width.
    #[test]
    fn a_polar_trace_does_not_collapse_the_x_axis() {
        let polar = Projection::centered_on(90.0);
        let (x, _) = polar.project(90.0, 100.0);
        assert!(x.is_finite());
        assert!(x != 0.0);
    }

    #[test]
    fn southern_latitudes_scale_the_same_as_northern() {
        let north = Projection::centered_on(45.0);
        let south = Projection::centered_on(-45.0);
        assert_eq!(north.project(0.0, 1.0), south.project(0.0, 1.0));
    }

    #[test]
    fn drawing_keeps_one_polyline_per_segment() {
        let trace = trace_of(vec![
            vec![(47.60, -122.33), (47.61, -122.34)],
            vec![(47.65, -122.40), (47.66, -122.41), (47.67, -122.42)],
        ]);
        let drawing = Drawing::of(&trace).unwrap();
        assert_eq!(drawing.segments.len(), 2);
        assert_eq!(drawing.segments[0].len(), 2);
        assert_eq!(drawing.segments[1].len(), 3);
    }

    #[test]
    fn drawing_bounds_cover_every_point() {
        let trace = trace_of(vec![vec![(47.60, -122.33), (47.70, -122.20)]]);
        let drawing = Drawing::of(&trace).unwrap();

        for segment in &drawing.segments {
            for (x, y) in segment {
                assert!(*x >= drawing.bounds.min_x - 1e-9 && *x <= drawing.bounds.max_x + 1e-9);
                assert!(*y >= drawing.bounds.min_y - 1e-9 && *y <= drawing.bounds.max_y + 1e-9);
            }
        }
        assert!(drawing.bounds.height() > 0.0);
        assert!(drawing.bounds.width() > 0.0);
    }

    #[test]
    fn endpoints_are_the_first_and_last_drawn_points() {
        let trace = trace_of(vec![
            vec![(47.60, -122.33), (47.61, -122.34)],
            vec![(47.65, -122.40), (47.66, -122.41)],
        ]);
        let drawing = Drawing::of(&trace).unwrap();
        let (start, end) = drawing.endpoints().unwrap();

        assert_eq!(start, drawing.projection.project(47.60, -122.33));
        assert_eq!(end, drawing.projection.project(47.66, -122.41));
    }

    #[test]
    fn an_empty_trace_has_nothing_to_draw() {
        assert!(Drawing::of(&Trace::default()).is_none());
        assert!(Drawing::of(&trace_of(vec![vec![]])).is_none());
    }
}
