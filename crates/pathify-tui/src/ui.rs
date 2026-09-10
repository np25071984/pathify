//! Drawing the map, the status line, and the help overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine, Points};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::app::App;
use super::input::{HELP, HINTS, HINTS_SHORT};
use pathify_core::Units;

const TRACK_COLOR: Color = Color::Cyan;
const START_COLOR: Color = Color::Green;
const END_COLOR: Color = Color::Red;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [map_area, status_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());

    draw_map(frame, app, map_area);
    draw_status(frame, app, status_area);

    if app.show_help {
        draw_help(frame, frame.area());
    }
}

fn draw_map(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", app.title));

    // The canvas draws inside the border, so the viewport is fitted to the
    // inner area — using the outer one would squash the map by two cells.
    let inner = block.inner(area);
    let viewport = app.viewport(inner.width, inner.height);
    let aspect = app.aspect();

    let segments = &app.drawing.segments;
    let endpoints = app.drawing.endpoints();

    let canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds(viewport.x_bounds(aspect))
        .y_bounds(viewport.y_bounds())
        .paint(move |ctx| {
            // Each segment is drawn on its own. Joining them would put a line
            // across a pause or a dropout — travel that never happened.
            for points in segments {
                for pair in points.windows(2) {
                    ctx.draw(&CanvasLine {
                        x1: pair[0].0,
                        y1: pair[0].1,
                        x2: pair[1].0,
                        y2: pair[1].1,
                        color: TRACK_COLOR,
                    });
                }
                // A one-point segment has no line, so plot it directly rather
                // than letting it disappear.
                if points.len() == 1 {
                    ctx.draw(&Points {
                        coords: points,
                        color: TRACK_COLOR,
                    });
                }
            }

            if let Some((start, end)) = endpoints {
                ctx.layer();
                ctx.draw(&Points {
                    coords: &[start],
                    color: START_COLOR,
                });
                ctx.draw(&Points {
                    coords: &[end],
                    color: END_COLOR,
                });
            }
        });

    frame.render_widget(canvas, area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let value = Style::default().fg(Color::Gray);

    // Most interesting first, because the tail is what gets dropped when the
    // terminal is narrow.
    let mut metrics = vec![format!("{} pts", app.summary.points)];
    metrics.push(format_distance(app.summary.distance_m, app.units));
    if let Some(elevation) = &app.summary.elevation {
        metrics.push(format_elevation(elevation.gain_m, app.units));
    }
    if let Some(width) = app.ground_width_m() {
        metrics.push(format!("{} across", format_distance(width, app.units)));
    }
    if let Some((lat, lon)) = app.center_coordinates() {
        metrics.push(format!("{lat:.4}, {lon:.4}"));
    }

    let (metrics, hints) = fit_status(&metrics, area.width as usize);

    let mut spans = Vec::new();
    for (i, metric) in metrics.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(SEPARATOR, dim));
        }
        spans.push(Span::styled((*metric).clone(), value));
    }
    if !hints.is_empty() {
        if !spans.is_empty() {
            spans.push(Span::styled("     ", dim));
        }
        spans.push(Span::styled(hints, dim));
    }

    frame.render_widget(Paragraph::new(TextLine::from(spans)), area);
}

const SEPARATOR: &str = "  ·  ";

/// Columns the separator occupies. `len()` would count bytes, and the middle
/// dot is two of them.
const SEPARATOR_WIDTH: usize = 5;

/// Blank columns between the metrics and the hints.
const HINT_GAP: usize = 5;

/// Decide what fits on the status line at `width` columns.
///
/// The key hints win over the metrics. They are the only place `q` and `?` are
/// written down, and a user who cannot see them has no way to discover how to
/// leave — whereas a missing distance readout costs nothing. On an ordinary
/// 80-column terminal the full status does not fit, so this is the common case
/// rather than an edge one.
pub fn fit_status(metrics: &[String], width: usize) -> (Vec<&String>, &'static str) {
    let hints = if HINTS.chars().count() + HINT_GAP <= width {
        HINTS
    } else if HINTS_SHORT.chars().count() <= width {
        HINTS_SHORT
    } else {
        ""
    };

    let mut used = hints.chars().count();

    let mut kept: Vec<&String> = Vec::new();
    for metric in metrics {
        // The gap before the hints is only drawn once there is something to
        // its left, so it is only charged for alongside the first metric.
        let leading = if kept.is_empty() {
            if hints.is_empty() { 0 } else { HINT_GAP }
        } else {
            SEPARATOR_WIDTH
        };
        let needed = leading + metric.chars().count();
        if used + needed > width {
            break;
        }
        used += needed;
        kept.push(metric);
    }
    (kept, hints)
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = 44u16.min(area.width.saturating_sub(4)).max(20);
    let height = (HELP.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let lines: Vec<TextLine> = HELP
        .iter()
        .map(|(keys, description)| {
            TextLine::from(vec![
                Span::styled(format!(" {keys:<14}"), key_style),
                Span::styled((*description).to_string(), Style::default()),
            ])
        })
        .collect();

    // Clear first: without it the map shows through the overlay.
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" keys ")),
        popup,
    );
}

/// Metres per foot, the conversion both imperial forms below are built on.
const METERS_PER_FOOT: f64 = 0.3048;

/// Metres per mile (5280 feet), i.e. the point at which imperial distances
/// switch from feet to miles.
const METERS_PER_MILE: f64 = METERS_PER_FOOT * 5280.0;

/// Meters below a kilometre, kilometres above it — or feet and miles at the
/// same thresholds, when the host prefers imperial.
///
/// A view 340 m across should say so rather than reporting `0.34 km`, and a
/// long ride should not be a five-digit meter count.
pub fn format_distance(meters: f64, units: Units) -> String {
    match units {
        Units::Metric => {
            if meters < 1000.0 {
                format!("{meters:.0} m")
            } else {
                format!("{:.2} km", meters / 1000.0)
            }
        }
        Units::Imperial => {
            if meters < METERS_PER_MILE {
                format!("{:.0} ft", meters / METERS_PER_FOOT)
            } else {
                format!("{:.2} mi", meters / METERS_PER_MILE)
            }
        }
    }
}

/// Elevation gain, always reported as a short-range reading: meters for
/// metric, feet for imperial. Unlike [`format_distance`] this never crosses
/// into kilometres or miles — a climb large enough for that would be an
/// error, not a real trace.
pub fn format_elevation(meters: f64, units: Units) -> String {
    match units {
        Units::Metric => format!("+{meters:.0} m"),
        Units::Imperial => format!("+{:.0} ft", meters / METERS_PER_FOOT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_switch_units_at_a_kilometre() {
        assert_eq!(format_distance(0.0, Units::Metric), "0 m");
        assert_eq!(format_distance(340.4, Units::Metric), "340 m");
        assert_eq!(format_distance(999.0, Units::Metric), "999 m");
        assert_eq!(format_distance(1000.0, Units::Metric), "1.00 km");
        assert_eq!(format_distance(2109.57, Units::Metric), "2.11 km");
    }

    #[test]
    fn imperial_distances_switch_units_at_a_mile() {
        assert_eq!(format_distance(0.0, Units::Imperial), "0 ft");
        assert_eq!(format_distance(304.8, Units::Imperial), "1000 ft");
        assert_eq!(format_distance(1609.344, Units::Imperial), "1.00 mi");
        assert_eq!(format_distance(3218.688, Units::Imperial), "2.00 mi");
    }

    #[test]
    fn elevation_follows_the_same_units() {
        assert_eq!(format_elevation(42.0, Units::Metric), "+42 m");
        assert_eq!(format_elevation(0.3048, Units::Imperial), "+1 ft");
    }

    /// Columns the status line actually occupies, counted the way a terminal
    /// does: characters, not bytes.
    fn rendered_width(kept: &[&String], hints: &str) -> usize {
        let metrics: usize = kept.iter().map(|m| m.chars().count()).sum::<usize>()
            + kept.len().saturating_sub(1) * SEPARATOR_WIDTH;
        match (kept.is_empty(), hints.is_empty()) {
            (_, true) => metrics,
            (true, false) => hints.chars().count(),
            (false, false) => metrics + HINT_GAP + hints.chars().count(),
        }
    }

    fn metrics() -> Vec<String> {
        [
            "14 pts",
            "3.80 km",
            "+42 m",
            "13.42 km across",
            "47.6300, -122.3125",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// The bug this guards: at 80 columns the full status overflows, and the
    /// naive layout cut the hints off mid-word — leaving no visible way to quit.
    #[test]
    fn the_quit_hint_survives_an_eighty_column_terminal() {
        let all = metrics();
        let (kept, hints) = fit_status(&all, 80);

        assert!(hints.contains("q quit"), "got {hints:?}");
        assert!(rendered_width(&kept, hints) <= 80);
    }

    #[test]
    fn a_wide_terminal_keeps_every_metric() {
        let all = metrics();
        let (kept, hints) = fit_status(&all, 200);
        assert_eq!(kept.len(), all.len());
        assert_eq!(hints, HINTS);
    }

    /// Metrics are dropped from the least interesting end first, so the point
    /// count outlives the centre coordinates.
    #[test]
    fn narrowing_drops_metrics_from_the_tail() {
        let all = metrics();
        let (wide, _) = fit_status(&all, 120);
        let (narrow, _) = fit_status(&all, 80);
        let (tiny, _) = fit_status(&all, 60);

        assert!(narrow.len() <= wide.len());
        assert!(tiny.len() <= narrow.len());
        if let Some(first) = tiny.first() {
            assert_eq!(*first, &all[0], "the point count should be the last to go");
        }
    }

    #[test]
    fn a_very_narrow_terminal_still_shows_how_to_quit() {
        let all = metrics();
        let (_, hints) = fit_status(&all, 20);
        assert_eq!(hints, HINTS_SHORT);
        assert!(hints.contains("quit"));
    }

    #[test]
    fn an_absurdly_narrow_terminal_does_not_panic() {
        let all = metrics();
        for width in [0, 1, 5, 10] {
            let (kept, _) = fit_status(&all, width);
            assert!(kept.len() <= all.len());
        }
    }

    /// Whatever the width, the status line must never be wider than it.
    #[test]
    fn the_status_line_never_overflows_its_width() {
        let all = metrics();
        for width in 0..140 {
            let (kept, hints) = fit_status(&all, width);
            assert!(
                rendered_width(&kept, hints) <= width.max(HINTS_SHORT.chars().count()),
                "at {width} columns the status rendered {} wide",
                rendered_width(&kept, hints)
            );
        }
    }
}
