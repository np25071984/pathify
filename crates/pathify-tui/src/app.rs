//! View state, and what each action does to it.
//!
//! The state machine is kept free of terminal calls so it can be driven
//! directly from tests: feed it actions, check where the view ended up.

use pathify_core::Trace;
use pathify_core::spatial::{DEFAULT_NOISE_THRESHOLD_M, distance, elevation_stats, trace_length};
use pathify_core::{Point, Summary};

use super::input::Action;
use super::projection::Drawing;
use super::units::Units;
use super::viewport::{PAN_STEP, Viewport, ZOOM_STEP, cell_aspect};

/// Whether the event loop should keep going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

pub struct App {
    pub title: String,
    pub drawing: Drawing,
    pub summary: Summary,
    pub show_help: bool,
    /// Distance units for the status line. Defaults to metric; the terminal
    /// entry point (`run`) overwrites this with [`Units::detect`] before the
    /// first draw, so a directly-constructed `App` — as every test builds one
    /// — stays deterministic.
    pub units: Units,
    /// `None` until the first draw, when the terminal size is finally known and
    /// the view can be fitted to it.
    viewport: Option<Viewport>,
    /// Aspect of the last drawn area, so key handling matches what is on screen.
    aspect: f64,
}

impl App {
    /// Build the view state, or `None` for a trace with nothing to draw.
    pub fn new(trace: &Trace, title: impl Into<String>) -> Option<Self> {
        Some(Self {
            title: title.into(),
            drawing: Drawing::of(trace)?,
            summary: Summary::of(trace, DEFAULT_NOISE_THRESHOLD_M),
            show_help: false,
            units: Units::default(),
            viewport: None,
            aspect: 1.0,
        })
    }

    /// The viewport, fitted to the drawing if this is the first look at it.
    ///
    /// Fitting is deferred because the right zoom depends on the shape of the
    /// terminal, which is not known until something is about to be drawn — and
    /// changes again whenever the window is resized.
    pub fn viewport(&mut self, width_cells: u16, height_cells: u16) -> Viewport {
        self.aspect = cell_aspect(width_cells, height_cells);
        *self
            .viewport
            .get_or_insert_with(|| Viewport::fit(&self.drawing.bounds, self.aspect))
    }

    pub fn aspect(&self) -> f64 {
        self.aspect
    }

    /// Apply an action, reporting whether the loop should continue.
    pub fn apply(&mut self, action: Action) -> Flow {
        let aspect = self.aspect;
        let viewport = self
            .viewport
            .get_or_insert_with(|| Viewport::fit(&self.drawing.bounds, aspect));

        match action {
            Action::Quit => return Flow::Exit,
            Action::PanLeft => viewport.pan(-PAN_STEP, 0.0, aspect),
            Action::PanRight => viewport.pan(PAN_STEP, 0.0, aspect),
            Action::PanUp => viewport.pan(0.0, PAN_STEP, aspect),
            Action::PanDown => viewport.pan(0.0, -PAN_STEP, aspect),
            Action::ZoomIn => viewport.zoom(1.0 / ZOOM_STEP),
            Action::ZoomOut => viewport.zoom(ZOOM_STEP),
            Action::Fit => *viewport = Viewport::fit(&self.drawing.bounds, aspect),
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Ignore => {}
        }
        Flow::Continue
    }

    /// Re-fit on the next draw, after the terminal has been resized.
    pub fn invalidate_fit(&mut self) {
        self.viewport = None;
    }

    /// Ground distance across the view, in meters.
    ///
    /// This is what makes the zoom level mean something: "0.4 km across" says
    /// more than any zoom percentage could.
    pub fn ground_width_m(&self) -> Option<f64> {
        let viewport = self.viewport?;
        let [min_x, max_x] = viewport.x_bounds(self.aspect);
        let (lat, west) = self.drawing.projection.unproject(min_x, viewport.center_y);
        let (_, east) = self.drawing.projection.unproject(max_x, viewport.center_y);

        let (Ok(a), Ok(b)) = (Point::new(lat, west), Point::new(lat, east)) else {
            return None;
        };
        Some(distance(&a, &b))
    }

    /// Where the middle of the view is, as latitude and longitude.
    pub fn center_coordinates(&self) -> Option<(f64, f64)> {
        let viewport = self.viewport?;
        Some(
            self.drawing
                .projection
                .unproject(viewport.center_x, viewport.center_y),
        )
    }

    /// Elevation gain, when the trace recorded any.
    pub fn elevation_gain_m(&self, trace: &Trace) -> Option<f64> {
        elevation_stats(trace, DEFAULT_NOISE_THRESHOLD_M).map(|stats| stats.gain_m)
    }

    pub fn distance_m(&self, trace: &Trace) -> f64 {
        trace_length(trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pathify_core::{Metadata, Segment, Track};

    fn trace() -> Trace {
        Trace {
            metadata: Metadata::default(),
            tracks: vec![Track {
                name: None,
                description: None,
                segments: vec![Segment::new(vec![
                    Point::new(47.60, -122.34).unwrap(),
                    Point::new(47.62, -122.30).unwrap(),
                ])],
            }],
        }
    }

    fn app() -> App {
        let mut app = App::new(&trace(), "ride.gpx").unwrap();
        app.viewport(80, 24); // as though a frame had been drawn
        app
    }

    #[test]
    fn a_trace_with_nothing_to_draw_has_no_view() {
        assert!(App::new(&Trace::default(), "empty.gpx").is_none());
    }

    #[test]
    fn the_view_starts_framed_on_the_whole_trace() {
        let mut app = app();
        let viewport = app.viewport(80, 24);
        let aspect = app.aspect();

        let [min_x, max_x] = viewport.x_bounds(aspect);
        let [min_y, max_y] = viewport.y_bounds();
        let bounds = app.drawing.bounds;
        assert!(min_x <= bounds.min_x && max_x >= bounds.max_x);
        assert!(min_y <= bounds.min_y && max_y >= bounds.max_y);
    }

    #[test]
    fn quitting_stops_the_loop_and_everything_else_does_not() {
        let mut app = app();
        assert_eq!(app.apply(Action::Quit), Flow::Exit);
        for action in [
            Action::PanLeft,
            Action::PanRight,
            Action::PanUp,
            Action::PanDown,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::Fit,
            Action::ToggleHelp,
            Action::Ignore,
        ] {
            assert_eq!(app.apply(action), Flow::Continue, "{action:?}");
        }
    }

    #[test]
    fn panning_opposite_directions_returns_to_the_start() {
        let mut app = app();
        let before = app.viewport(80, 24);

        app.apply(Action::PanLeft);
        app.apply(Action::PanRight);
        app.apply(Action::PanUp);
        app.apply(Action::PanDown);

        let after = app.viewport(80, 24);
        assert!((before.center_x - after.center_x).abs() < 1e-9);
        assert!((before.center_y - after.center_y).abs() < 1e-9);
    }

    #[test]
    fn panning_up_increases_latitude() {
        let mut app = app();
        let before = app.viewport(80, 24).center_y;
        app.apply(Action::PanUp);
        assert!(app.viewport(80, 24).center_y > before, "up should go north");
    }

    #[test]
    fn zooming_in_shows_less_ground() {
        let mut app = app();
        let before = app.ground_width_m().unwrap();
        app.apply(Action::ZoomIn);
        let after = app.ground_width_m().unwrap();
        assert!(after < before, "{after} should be less than {before}");
    }

    /// Fit is the way back after getting lost, so it has to work from anywhere.
    #[test]
    fn fit_recovers_the_view_after_panning_far_away() {
        let mut app = app();
        let original = app.viewport(80, 24);

        for _ in 0..50 {
            app.apply(Action::PanRight);
            app.apply(Action::ZoomIn);
        }
        app.apply(Action::Fit);

        let restored = app.viewport(80, 24);
        assert!((restored.center_x - original.center_x).abs() < 1e-9);
        assert!((restored.span_y - original.span_y).abs() < 1e-9);
    }

    #[test]
    fn help_toggles() {
        let mut app = app();
        assert!(!app.show_help);
        app.apply(Action::ToggleHelp);
        assert!(app.show_help);
        app.apply(Action::ToggleHelp);
        assert!(!app.show_help);
    }

    #[test]
    fn resizing_refits_the_view() {
        let mut app = app();
        let wide = app.viewport(200, 20);
        app.invalidate_fit();
        let tall = app.viewport(40, 60);
        assert_ne!(wide.span_y, tall.span_y, "a reshaped terminal should refit");
    }

    #[test]
    fn the_view_reports_where_it_is_and_how_much_it_shows() {
        let mut app = app();
        app.viewport(80, 24);

        let (lat, lon) = app.center_coordinates().unwrap();
        assert!((lat - 47.61).abs() < 0.01, "got {lat}");
        assert!((lon - -122.32).abs() < 0.01, "got {lon}");

        let width = app.ground_width_m().unwrap();
        assert!(width > 0.0 && width.is_finite());
    }

    /// Zoom is clamped, so even leaning on the key must not produce a view of
    /// zero or infinite width.
    #[test]
    fn extreme_zooming_keeps_the_view_finite() {
        let mut app = app();
        for _ in 0..400 {
            app.apply(Action::ZoomIn);
        }
        assert!(app.ground_width_m().unwrap() > 0.0);

        for _ in 0..800 {
            app.apply(Action::ZoomOut);
        }
        let width = app.ground_width_m().unwrap();
        assert!(width.is_finite() && width > 0.0);
    }
}
