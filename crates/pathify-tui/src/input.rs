//! Turning key presses into intentions.
//!
//! Kept separate from the app so the keymap can be tested by feeding it key
//! events, with no terminal and no event loop involved.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
    /// Frame the whole trace again.
    Fit,
    ToggleHelp,
    /// A key with no meaning here; the view is left alone.
    Ignore,
}

/// Map a key press to an action.
///
/// Both arrow keys and `hjkl` pan, so the view works for people who expect
/// either. Zoom answers to `+`/`-` and to `i`/`o`.
pub fn action_for(key: KeyEvent) -> Action {
    // A key that is only being released or held should not act twice; Windows
    // reports those kinds where Unix terminals do not.
    if key.kind == KeyEventKind::Release {
        return Action::Ignore;
    }

    // Ctrl-C means quit here as much as anywhere, and raw mode means the
    // terminal will not deliver it as a signal.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Left | KeyCode::Char('h') => Action::PanLeft,
        KeyCode::Right | KeyCode::Char('l') => Action::PanRight,
        KeyCode::Up | KeyCode::Char('k') => Action::PanUp,
        KeyCode::Down | KeyCode::Char('j') => Action::PanDown,
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('i') => Action::ZoomIn,
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('o') => Action::ZoomOut,
        KeyCode::Char('f') | KeyCode::Char('r') => Action::Fit,
        KeyCode::Char('?') => Action::ToggleHelp,
        _ => Action::Ignore,
    }
}

/// The key hints shown along the bottom of the screen.
pub const HINTS: &str = "hjkl pan · +/- zoom · f fit · ? help · q quit";

/// Hints for a terminal too narrow for the full set.
///
/// Whatever else is dropped, how to get help and how to get out have to stay:
/// they are the only way out of a full-screen program the user may not have
/// meant to open.
pub const HINTS_SHORT: &str = "? help · q quit";

/// Rows for the help overlay: the key, and what it does.
pub const HELP: &[(&str, &str)] = &[
    ("h j k l", "pan left, down, up, right"),
    ("← ↓ ↑ →", "pan"),
    ("+ = i", "zoom in"),
    ("- _ o", "zoom out"),
    ("f r", "fit the whole trace"),
    ("?", "show or hide this help"),
    ("q Esc Ctrl-C", "quit"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn arrows_and_hjkl_pan_the_same_way() {
        assert_eq!(action_for(press(KeyCode::Left)), Action::PanLeft);
        assert_eq!(action_for(press(KeyCode::Char('h'))), Action::PanLeft);
        assert_eq!(action_for(press(KeyCode::Right)), Action::PanRight);
        assert_eq!(action_for(press(KeyCode::Char('l'))), Action::PanRight);
        assert_eq!(action_for(press(KeyCode::Up)), Action::PanUp);
        assert_eq!(action_for(press(KeyCode::Char('k'))), Action::PanUp);
        assert_eq!(action_for(press(KeyCode::Down)), Action::PanDown);
        assert_eq!(action_for(press(KeyCode::Char('j'))), Action::PanDown);
    }

    #[test]
    fn zoom_accepts_both_spellings() {
        for code in [KeyCode::Char('+'), KeyCode::Char('='), KeyCode::Char('i')] {
            assert_eq!(action_for(press(code)), Action::ZoomIn, "{code:?}");
        }
        for code in [KeyCode::Char('-'), KeyCode::Char('_'), KeyCode::Char('o')] {
            assert_eq!(action_for(press(code)), Action::ZoomOut, "{code:?}");
        }
    }

    #[test]
    fn there_is_always_a_way_out() {
        assert_eq!(action_for(press(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(action_for(press(KeyCode::Esc)), Action::Quit);
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit,
            "raw mode swallows the signal, so Ctrl-C has to be handled here"
        );
    }

    #[test]
    fn unknown_keys_do_nothing() {
        assert_eq!(action_for(press(KeyCode::Char('z'))), Action::Ignore);
        assert_eq!(action_for(press(KeyCode::Tab)), Action::Ignore);
    }

    /// A key release should not repeat the action its press already performed.
    #[test]
    fn key_releases_are_ignored() {
        let mut release = press(KeyCode::Char('j'));
        release.kind = KeyEventKind::Release;
        assert_eq!(action_for(release), Action::Ignore);
    }

    #[test]
    fn every_documented_key_is_actually_bound() {
        for (keys, _) in HELP {
            for key in keys.split_whitespace() {
                let Some(code) = single_char(key) else {
                    continue;
                };
                assert_ne!(
                    action_for(press(code)),
                    Action::Ignore,
                    "help lists `{key}` but nothing is bound to it"
                );
            }
        }
    }

    fn single_char(key: &str) -> Option<KeyCode> {
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii() => Some(KeyCode::Char(c)),
            _ => None,
        }
    }
}
