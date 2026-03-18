pub mod theme;
pub mod views;
pub mod widgets;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    self, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

pub type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// Initialize the terminal for TUI rendering.
pub fn init() -> Result<Term> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
pub fn restore(terminal: &mut Term) -> Result<()> {
    terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Check if a key event is a quit command.
pub fn is_quit(key: &KeyEvent) -> bool {
    matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            ..
        } | KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}

/// Poll for the next key event (non-blocking with timeout).
pub fn next_key_event(timeout: std::time::Duration) -> Result<Option<KeyEvent>> {
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            return Ok(Some(key));
        }
    }
    Ok(None)
}
