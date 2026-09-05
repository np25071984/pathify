//! Terminal map view for Pathify.
//!
//! Kept as its own crate so `ratatui` and `crossterm` never enter the
//! dependency graph of the one-shot, pipeline-friendly commands, and so the
//! spatial logic in `pathify-core` stays testable without a terminal.
//!
//! The interesting parts — the projection, the viewport, the keymap, and the
//! state machine — are plain functions over plain data, so nearly all of this
//! crate is tested without a terminal anywhere in sight. Only [`run`] itself
//! touches one.

pub mod app;
pub mod input;
pub mod projection;
pub mod ui;
pub mod viewport;

use std::io;

use crossterm::event::{self, Event};
use pathify_core::Trace;

pub use app::{App, Flow};
pub use input::Action;
pub use projection::{Drawing, Projection};
pub use viewport::Viewport;

/// Show a trace on an interactive map until the user quits.
///
/// This takes over the terminal, so the caller is responsible for checking
/// there is one to take over — see `is_interactive`.
///
/// Returns `Ok(false)` when the trace had nothing to draw, so the caller can
/// say so plainly rather than presenting an empty screen.
pub fn run(trace: &Trace, title: &str) -> io::Result<bool> {
    let Some(mut app) = App::new(trace, title) else {
        return Ok(false);
    };

    // `try_init` installs a panic hook that restores the terminal first, so a
    // crash in here cannot leave the user with a broken shell.
    let mut terminal = ratatui::try_init()?;
    let outcome = event_loop(&mut terminal, &mut app);
    ratatui::try_restore()?;

    outcome.map(|()| true)
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Blocking: a map does not animate, so there is nothing to do between
        // key presses and no reason to spin a CPU waiting for one.
        match event::read()? {
            Event::Key(key) => {
                if app.apply(input::action_for(key)) == Flow::Exit {
                    return Ok(());
                }
            }
            // The right zoom depends on the shape of the window, so a resize
            // reframes rather than stretching what was already there.
            Event::Resize(_, _) => app.invalidate_fit(),
            _ => {}
        }
    }
}

/// Whether standard output is a terminal.
///
/// `view` is the one command that cannot be piped: it paints a screen rather
/// than emitting a document, and redirecting that into a file would produce
/// nothing anyone wants. Input may still arrive on a pipe — key presses are
/// read from the controlling terminal, not from stdin.
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    io::stdout().is_terminal()
}
