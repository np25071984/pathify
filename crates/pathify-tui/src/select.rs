//! A multi-select menu.
//!
//! `takeout` needs the user to pick which activity types to pull out of an
//! archive, and the unit of choice is a type rather than a file — an archive
//! holds hundreds of recordings spread across per-day files that do not
//! correspond to activities at all. So this is a list with checkboxes, not a
//! file picker.
//!
//! It lives here rather than in the CLI so that Pathify has one terminal
//! stack. As with the map, everything except [`choose`] is a plain function
//! over plain data and is tested with no terminal in sight.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub use crate::app::Flow;

const HINTS: &str = "↑/↓ move · space toggle · a all · enter confirm · q cancel";

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What the row is, e.g. `Walk`.
    pub label: String,
    /// The counts beside it, e.g. `(117 with GPS, 27 without)`.
    pub detail: String,
    pub selected: bool,
    /// Rows that cannot produce anything are shown, so the user can see the
    /// archive was read correctly, but they cannot be ticked.
    pub enabled: bool,
}

impl Choice {
    pub fn new(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            selected: false,
            enabled: true,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// What a key press means in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Toggle,
    /// Tick everything, or untick it if everything is already ticked.
    ToggleAll,
    Confirm,
    Cancel,
    Ignore,
}

/// Map a key press to an action.
pub fn action_for(key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::Ignore;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return Action::Cancel;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Char(' ') | KeyCode::Char('x') => Action::Toggle,
        KeyCode::Char('a') => Action::ToggleAll,
        KeyCode::Enter => Action::Confirm,
        KeyCode::Char('q') | KeyCode::Esc => Action::Cancel,
        _ => Action::Ignore,
    }
}

/// How the menu ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Confirmed,
    Cancelled,
}

/// The menu's state.
#[derive(Debug, Clone)]
pub struct Select {
    pub title: String,
    pub choices: Vec<Choice>,
    pub cursor: usize,
    pub outcome: Option<Outcome>,
}

impl Select {
    pub fn new(title: impl Into<String>, choices: Vec<Choice>) -> Self {
        // Start on something that can actually be ticked, so the first press
        // of space does what it looks like it will.
        let cursor = choices.iter().position(|c| c.enabled).unwrap_or(0);
        Self {
            title: title.into(),
            choices,
            cursor,
            outcome: None,
        }
    }

    pub fn apply(&mut self, action: Action) -> Flow {
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::Toggle => {
                if let Some(choice) = self.choices.get_mut(self.cursor)
                    && choice.enabled
                {
                    choice.selected = !choice.selected;
                }
            }
            Action::ToggleAll => {
                let target = !self
                    .choices
                    .iter()
                    .filter(|c| c.enabled)
                    .all(|c| c.selected);
                for choice in self.choices.iter_mut().filter(|c| c.enabled) {
                    choice.selected = target;
                }
            }
            Action::Confirm => {
                self.outcome = Some(Outcome::Confirmed);
                return Flow::Exit;
            }
            Action::Cancel => {
                self.outcome = Some(Outcome::Cancelled);
                return Flow::Exit;
            }
            Action::Ignore => {}
        }
        Flow::Continue
    }

    /// Indexes of the ticked rows.
    pub fn chosen(&self) -> Vec<usize> {
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.selected)
            .map(|(index, _)| index)
            .collect()
    }

    /// Move by `step` rows, skipping ones that cannot be ticked and stopping
    /// at the ends rather than wrapping.
    fn move_cursor(&mut self, step: isize) {
        let mut at = self.cursor as isize;
        loop {
            at += step;
            match self.choices.get(usize::try_from(at).unwrap_or(usize::MAX)) {
                Some(choice) if choice.enabled => {
                    self.cursor = at as usize;
                    return;
                }
                Some(_) => continue,
                None => return,
            }
        }
    }

    /// Width the label column is padded to, so the counts line up.
    fn label_width(&self) -> usize {
        self.choices
            .iter()
            .map(|choice| choice.label.chars().count())
            .max()
            .unwrap_or(0)
    }
}

/// Ask the user to pick from a list, returning the indexes they ticked.
///
/// `None` means they cancelled.
///
/// This takes over the terminal, so the caller is responsible for having
/// checked there is one to take over — and, because it paints a screen, that
/// stdout is not being redirected into something a screen would ruin.
pub fn choose(title: &str, choices: Vec<Choice>) -> io::Result<Option<Vec<usize>>> {
    let mut select = Select::new(title, choices);

    let mut terminal = ratatui::try_init()?;
    let outcome = event_loop(&mut terminal, &mut select);
    ratatui::try_restore()?;
    outcome?;

    Ok(match select.outcome {
        Some(Outcome::Confirmed) => Some(select.chosen()),
        _ => None,
    })
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, select: &mut Select) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, select))?;
        if let crossterm::event::Event::Key(key) = crossterm::event::read()?
            && select.apply(action_for(key)) == Flow::Exit
        {
            return Ok(());
        }
    }
}

fn draw(frame: &mut Frame, select: &Select) {
    let [title_area, list_area, hint_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_title(frame, select, title_area);
    draw_list(frame, select, list_area);
    frame.render_widget(
        Paragraph::new(Span::styled(HINTS, Style::default().fg(Color::DarkGray))),
        hint_area,
    );
}

fn draw_title(frame: &mut Frame, select: &Select, area: Rect) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                select.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ]),
        area,
    );
}

fn draw_list(frame: &mut Frame, select: &Select, area: Rect) {
    let width = select.label_width();
    let items: Vec<ListItem> = select
        .choices
        .iter()
        .map(|choice| {
            let box_style = if choice.enabled {
                Style::default().fg(if choice.selected {
                    Color::Green
                } else {
                    Color::Gray
                })
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let text_style = if choice.enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(vec![
                Span::styled(if choice.selected { "[x] " } else { "[ ] " }, box_style),
                Span::styled(format!("{:width$}  ", choice.label), text_style),
                Span::styled(choice.detail.clone(), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let mut state = ListState::default().with_selected(Some(select.cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Select {
        Select::new(
            "pick",
            vec![
                Choice::new("Walk", "(117 with GPS, 27 without)"),
                Choice::new("Swim", "(0 with GPS, 12 without)").enabled(false),
                Choice::new("Bike", "(1 with GPS, 0 without)"),
            ],
        )
    }

    #[test]
    fn the_cursor_starts_on_something_that_can_be_ticked() {
        let choices = vec![
            Choice::new("Swim", "").enabled(false),
            Choice::new("Walk", ""),
        ];
        assert_eq!(Select::new("pick", choices).cursor, 1);
    }

    /// A type with no GPS is listed so the user can see it was found, but
    /// ticking it would promise a track that cannot exist.
    #[test]
    fn rows_that_cannot_produce_a_track_are_skipped_and_cannot_be_ticked() {
        let mut select = menu();
        assert_eq!(select.cursor, 0);
        select.apply(Action::Down);
        assert_eq!(
            select.cursor, 2,
            "the Swim row should have been stepped over"
        );
        select.apply(Action::Down);
        assert_eq!(
            select.cursor, 2,
            "the cursor stops at the end, it does not wrap"
        );

        select.cursor = 1;
        select.apply(Action::Toggle);
        assert!(!select.choices[1].selected);
    }

    #[test]
    fn space_toggles_and_enter_confirms() {
        let mut select = menu();
        assert_eq!(select.apply(Action::Toggle), Flow::Continue);
        assert_eq!(select.apply(Action::Confirm), Flow::Exit);
        assert_eq!(select.outcome, Some(Outcome::Confirmed));
        assert_eq!(select.chosen(), vec![0]);
    }

    #[test]
    fn toggling_all_covers_only_the_rows_that_can_be_ticked() {
        let mut select = menu();
        select.apply(Action::ToggleAll);
        assert_eq!(select.chosen(), vec![0, 2]);
        select.apply(Action::ToggleAll);
        assert!(select.chosen().is_empty());
    }

    #[test]
    fn cancelling_returns_nothing_however_much_was_ticked() {
        let mut select = menu();
        select.apply(Action::Toggle);
        assert_eq!(select.apply(Action::Cancel), Flow::Exit);
        assert_eq!(select.outcome, Some(Outcome::Cancelled));
    }

    #[test]
    fn the_keymap_covers_both_arrows_and_vi_keys() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert_eq!(action_for(key(KeyCode::Up)), Action::Up);
        assert_eq!(action_for(key(KeyCode::Char('j'))), Action::Down);
        assert_eq!(action_for(key(KeyCode::Char(' '))), Action::Toggle);
        assert_eq!(action_for(key(KeyCode::Enter)), Action::Confirm);
        assert_eq!(action_for(key(KeyCode::Esc)), Action::Cancel);
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Cancel
        );
        assert_eq!(action_for(key(KeyCode::Char('z'))), Action::Ignore);
    }

    #[test]
    fn labels_are_padded_so_the_counts_line_up() {
        assert_eq!(menu().label_width(), 4);
    }
}
