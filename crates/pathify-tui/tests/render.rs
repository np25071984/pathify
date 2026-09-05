//! Rendering tests for the map.
//!
//! These draw into an in-memory buffer rather than a terminal, so what the user
//! would actually see can be asserted in CI — the braille track, the title, the
//! status line, and the help overlay — without a pty anywhere.

use pathify_core::{Metadata, Point, Segment, Trace, Track};
use pathify_tui::app::App;
use pathify_tui::input::Action;
use pathify_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn trace() -> Trace {
    // A short diagonal run, then a second segment after a gap.
    let leg = |from: (f64, f64), steps: usize| -> Segment {
        Segment::new(
            (0..steps)
                .map(|i| Point::new(from.0 + i as f64 * 0.002, from.1 + i as f64 * 0.003).unwrap())
                .collect(),
        )
    };
    Trace {
        metadata: Metadata::default(),
        tracks: vec![Track {
            name: Some("Outbound".into()),
            description: None,
            segments: vec![leg((47.60, -122.34), 8), leg((47.65, -122.30), 6)],
        }],
    }
}

/// Render one frame and return the screen as text, one line per row.
fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

fn braille_count(screen: &[String]) -> usize {
    screen
        .iter()
        .flat_map(|line| line.chars())
        .filter(|c| ('\u{2800}'..='\u{28FF}').contains(c) && *c != '\u{2800}')
        .count()
}

#[test]
fn draws_the_track_as_braille_dots() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let screen = render(&mut app, 80, 24);

    assert!(
        braille_count(&screen) >= 15,
        "expected a drawn track, got {} braille cells:\n{}",
        braille_count(&screen),
        screen.join("\n")
    );
}

#[test]
fn shows_the_file_name_and_the_status_line() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let text = render(&mut app, 80, 24).join("\n");

    assert!(text.contains("ride.gpx"), "title missing:\n{text}");
    assert!(text.contains("14 pts"), "point count missing:\n{text}");
    assert!(text.contains("q quit"), "key hints missing:\n{text}");
}

/// An 80-column terminal cannot hold every metric, and the hints are the ones
/// that must survive — a user who cannot see `q` cannot leave.
#[test]
fn a_narrow_terminal_keeps_the_hints_over_the_metrics() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let narrow = render(&mut app, 80, 24).join("\n");

    assert!(narrow.contains("q quit"), "{narrow}");
    assert!(
        !narrow.contains("across"),
        "the scale should have yielded to the hints:\n{narrow}"
    );
}

/// Given room, everything is shown.
#[test]
fn a_wide_terminal_shows_the_scale_and_position() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let wide = render(&mut app, 140, 24).join("\n");

    assert!(wide.contains("across"), "scale missing:\n{wide}");
    assert!(wide.contains("47.6"), "centre position missing:\n{wide}");
    assert!(wide.contains("q quit"), "{wide}");
}

/// The status line reports where the view is, so it has to track panning.
#[test]
fn the_status_line_follows_the_view() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let before = render(&mut app, 80, 24).join("\n");

    for _ in 0..3 {
        app.apply(Action::PanRight);
    }
    let after = render(&mut app, 80, 24).join("\n");

    assert_ne!(before, after, "panning did not change the screen");
}

#[test]
fn zooming_in_redraws_at_a_smaller_scale() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    render(&mut app, 80, 24);
    let wide = app.ground_width_m().unwrap();

    app.apply(Action::ZoomIn);
    render(&mut app, 80, 24);
    let close = app.ground_width_m().unwrap();

    assert!(close < wide, "{close} should be less than {wide}");
}

#[test]
fn the_help_overlay_appears_and_covers_the_map() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let without = render(&mut app, 80, 24).join("\n");
    assert!(!without.contains("fit the whole trace"));

    app.apply(Action::ToggleHelp);
    let with = render(&mut app, 80, 24).join("\n");

    assert!(with.contains("keys"), "overlay title missing:\n{with}");
    assert!(with.contains("fit the whole trace"), "{with}");
    assert!(with.contains("pan left, down, up, right"), "{with}");

    app.apply(Action::ToggleHelp);
    assert!(
        !render(&mut app, 80, 24)
            .join("\n")
            .contains("fit the whole trace")
    );
}

/// The whole trace should be on screen at the start, without panning.
#[test]
fn the_initial_view_frames_the_whole_trace() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    let fitted = braille_count(&render(&mut app, 80, 24));

    // Zoom well in, and strictly less of the track can be visible.
    for _ in 0..6 {
        app.apply(Action::ZoomIn);
    }
    let zoomed = braille_count(&render(&mut app, 80, 24));

    assert!(
        zoomed < fitted,
        "zoomed view shows {zoomed} cells, fitted shows {fitted}"
    );
}

/// A very small terminal must not panic — the layout has to survive a window
/// that barely has room for the border.
#[test]
fn a_tiny_terminal_still_renders() {
    let mut app = App::new(&trace(), "ride.gpx").unwrap();
    for (width, height) in [(20, 5), (10, 4), (4, 3)] {
        let screen = render(&mut app, width, height);
        assert_eq!(screen.len(), height as usize);
    }
}

#[test]
fn a_single_point_trace_still_draws_something() {
    let single = Trace {
        metadata: Metadata::default(),
        tracks: vec![Track {
            name: None,
            description: None,
            segments: vec![Segment::new(vec![Point::new(47.6, -122.3).unwrap()])],
        }],
    };

    let mut app = App::new(&single, "point.gpx").unwrap();
    let screen = render(&mut app, 80, 24);
    assert!(
        braille_count(&screen) > 0,
        "a lone point vanished:\n{}",
        screen.join("\n")
    );
}
