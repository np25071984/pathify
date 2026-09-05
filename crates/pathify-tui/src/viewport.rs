//! The window onto the trace: where the view is centred and how much it shows.
//!
//! All of it is plain arithmetic on numbers, deliberately: the panning and
//! zooming rules are the part of a TUI most likely to be subtly wrong, and this
//! way they can be tested without a terminal anywhere in sight.

use super::projection::PlanarBounds;

/// Fraction of the visible span a single pan step moves.
pub const PAN_STEP: f64 = 0.15;

/// How much one zoom step changes the visible span.
pub const ZOOM_STEP: f64 = 1.4;

/// Closest zoom, in degrees of latitude across the view — about a meter.
pub const MIN_SPAN: f64 = 0.00001;

/// Widest zoom: the whole planet, and no further.
pub const MAX_SPAN: f64 = 180.0;

/// Empty margin left around a trace when the view is fitted to it.
const FIT_PADDING: f64 = 1.08;

/// Span used for a trace with no extent at all — a single point, or one
/// recorded without moving. Roughly 100 m across, so the marker has some room.
const DEGENERATE_SPAN: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub center_x: f64,
    pub center_y: f64,
    /// Vertical extent in projected units, i.e. degrees of latitude.
    pub span_y: f64,
}

/// Ratio of horizontal to vertical projected units per terminal cell.
///
/// A terminal cell is about twice as tall as it is wide, and braille divides it
/// into 2 dots across by 4 down — so a braille dot comes out very nearly
/// square. Getting equal ground distance per dot in both axes therefore means
/// `span_x / 2·width == span_y / 4·height`, which is the ratio returned here.
/// Skipping this is what makes hand-rolled terminal maps look squashed.
pub fn cell_aspect(width_cells: u16, height_cells: u16) -> f64 {
    let width = width_cells.max(1) as f64;
    let height = height_cells.max(1) as f64;
    width / (2.0 * height)
}

impl Viewport {
    /// Frame the whole trace, with a little margin.
    pub fn fit(bounds: &PlanarBounds, aspect: f64) -> Self {
        let (center_x, center_y) = bounds.center();

        // The view has to cover the trace's height, and enough height that its
        // width fits too once the aspect ratio is applied.
        let needed = bounds
            .height()
            .max(bounds.width() / aspect.max(f64::EPSILON));
        let span_y = if needed.is_finite() && needed > 0.0 {
            (needed * FIT_PADDING).clamp(MIN_SPAN, MAX_SPAN)
        } else {
            DEGENERATE_SPAN
        };

        Self {
            center_x,
            center_y,
            span_y,
        }
    }

    pub fn span_x(&self, aspect: f64) -> f64 {
        self.span_y * aspect
    }

    pub fn x_bounds(&self, aspect: f64) -> [f64; 2] {
        let half = self.span_x(aspect) / 2.0;
        [self.center_x - half, self.center_x + half]
    }

    pub fn y_bounds(&self) -> [f64; 2] {
        let half = self.span_y / 2.0;
        [self.center_y - half, self.center_y + half]
    }

    /// Zoom by `factor`: below one moves closer, above one pulls back.
    ///
    /// Clamped at both ends so repeated keypresses cannot reach a zero or
    /// infinite span, which would divide the rendering by nothing.
    pub fn zoom(&mut self, factor: f64) {
        self.span_y = (self.span_y * factor).clamp(MIN_SPAN, MAX_SPAN);
    }

    /// Move the view by a fraction of what it currently shows.
    ///
    /// Panning in fractions rather than fixed degrees keeps the gesture feeling
    /// the same at every zoom level: one press always moves the same visible
    /// proportion, whether the view spans a street or a country.
    pub fn pan(&mut self, fraction_x: f64, fraction_y: f64, aspect: f64) {
        self.center_x += fraction_x * self.span_x(aspect);
        self.center_y += fraction_y * self.span_y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> PlanarBounds {
        PlanarBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// A wide terminal shows proportionally more longitude than latitude.
    #[test]
    fn cell_aspect_accounts_for_tall_cells_and_braille_dots() {
        // 80 columns by 20 rows: 80 / (2 * 20) = 2.0
        assert_eq!(cell_aspect(80, 20), 2.0);
        // A square-ish cell count is still wider than tall in ground units.
        assert_eq!(cell_aspect(40, 20), 1.0);
        // Never divides by zero on a degenerate area.
        assert!(cell_aspect(0, 0).is_finite());
    }

    #[test]
    fn fitting_centres_on_the_trace() {
        let viewport = Viewport::fit(&bounds(-2.0, 10.0, 2.0, 14.0), 1.0);
        assert_eq!(viewport.center_x, 0.0);
        assert_eq!(viewport.center_y, 12.0);
    }

    #[test]
    fn fitting_covers_the_whole_trace_with_margin() {
        let area = bounds(-2.0, 10.0, 2.0, 14.0);
        let aspect = 1.0;
        let viewport = Viewport::fit(&area, aspect);

        let [min_x, max_x] = viewport.x_bounds(aspect);
        let [min_y, max_y] = viewport.y_bounds();
        assert!(min_x <= area.min_x && max_x >= area.max_x);
        assert!(min_y <= area.min_y && max_y >= area.max_y);
    }

    /// A trace wider than it is tall must be fitted on its width, or the ends
    /// of the route hang off the sides of the screen.
    #[test]
    fn fitting_a_wide_trace_uses_its_width() {
        let wide = bounds(-10.0, 0.0, 10.0, 0.5);
        let aspect = 2.0;
        let viewport = Viewport::fit(&wide, aspect);

        let [min_x, max_x] = viewport.x_bounds(aspect);
        assert!(min_x <= -10.0 && max_x >= 10.0, "the width did not fit");
    }

    #[test]
    fn fitting_a_single_point_still_gives_a_usable_view() {
        let point = bounds(1.0, 2.0, 1.0, 2.0);
        let viewport = Viewport::fit(&point, 2.0);
        assert!(viewport.span_y > 0.0);
        assert!(viewport.span_y.is_finite());
        assert_eq!(viewport.center_x, 1.0);
        assert_eq!(viewport.center_y, 2.0);
    }

    #[test]
    fn zooming_in_and_out_are_inverses() {
        let mut viewport = Viewport::fit(&bounds(-1.0, -1.0, 1.0, 1.0), 1.0);
        let original = viewport.span_y;

        viewport.zoom(1.0 / ZOOM_STEP);
        assert!(viewport.span_y < original);
        viewport.zoom(ZOOM_STEP);
        assert!((viewport.span_y - original).abs() < 1e-9);
    }

    /// Holding a zoom key must not reach zero or infinity, either of which
    /// makes the next render divide by nothing.
    #[test]
    fn zoom_is_clamped_at_both_extremes() {
        let mut viewport = Viewport::fit(&bounds(-1.0, -1.0, 1.0, 1.0), 1.0);

        for _ in 0..500 {
            viewport.zoom(1.0 / ZOOM_STEP);
        }
        assert_eq!(viewport.span_y, MIN_SPAN);

        for _ in 0..500 {
            viewport.zoom(ZOOM_STEP);
        }
        assert_eq!(viewport.span_y, MAX_SPAN);
    }

    #[test]
    fn zooming_does_not_move_the_centre() {
        let mut viewport = Viewport::fit(&bounds(-1.0, 5.0, 3.0, 9.0), 1.5);
        let (x, y) = (viewport.center_x, viewport.center_y);

        viewport.zoom(1.0 / ZOOM_STEP);
        viewport.zoom(ZOOM_STEP * 3.0);
        assert_eq!((viewport.center_x, viewport.center_y), (x, y));
    }

    #[test]
    fn panning_moves_by_a_fraction_of_the_visible_span() {
        let aspect = 2.0;
        let mut viewport = Viewport {
            center_x: 0.0,
            center_y: 0.0,
            span_y: 1.0,
        };

        viewport.pan(PAN_STEP, 0.0, aspect);
        assert!((viewport.center_x - PAN_STEP * 2.0).abs() < 1e-9);

        viewport.pan(0.0, -PAN_STEP, aspect);
        assert!((viewport.center_y + PAN_STEP).abs() < 1e-9);
    }

    /// Panning in fractions is what keeps the gesture consistent: a keypress
    /// should cover the same share of the screen at every zoom level.
    #[test]
    fn a_pan_step_covers_the_same_screen_share_at_any_zoom() {
        let aspect = 1.0;
        let mut close = Viewport {
            center_x: 0.0,
            center_y: 0.0,
            span_y: 0.01,
        };
        let mut far = Viewport {
            center_x: 0.0,
            center_y: 0.0,
            span_y: 10.0,
        };

        close.pan(0.0, PAN_STEP, aspect);
        far.pan(0.0, PAN_STEP, aspect);

        assert!((close.center_y / close.span_y - far.center_y / far.span_y).abs() < 1e-9);
    }

    #[test]
    fn panning_and_returning_restores_the_centre() {
        let aspect = 1.7;
        let mut viewport = Viewport {
            center_x: 4.0,
            center_y: -3.0,
            span_y: 2.0,
        };

        viewport.pan(PAN_STEP, PAN_STEP, aspect);
        viewport.pan(-PAN_STEP, -PAN_STEP, aspect);
        assert!((viewport.center_x - 4.0).abs() < 1e-9);
        assert!((viewport.center_y + 3.0).abs() < 1e-9);
    }
}
