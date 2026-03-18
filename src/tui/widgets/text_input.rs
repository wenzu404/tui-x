use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::tui::theme::Theme;

/// A simple text input widget with cursor.
pub struct TextInput {
    pub content: String,
    pub cursor: usize,
    pub placeholder: String,
    pub multiline: bool,
}

impl TextInput {
    pub fn new(placeholder: &str) -> Self {
        Self {
            content: String::new(),
            cursor: 0,
            placeholder: placeholder.to_string(),
            multiline: true,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Char(c) => {
                self.content.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputAction::Changed
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    // Find previous char boundary
                    let prev = self.content[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.content.drain(prev..self.cursor);
                    self.cursor = prev;
                    InputAction::Changed
                } else {
                    InputAction::None
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.content.len() {
                    let next = self.content[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.content.len());
                    self.content.drain(self.cursor..next);
                    InputAction::Changed
                } else {
                    InputAction::None
                }
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = self.content[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
                InputAction::None
            }
            KeyCode::Right => {
                if self.cursor < self.content.len() {
                    self.cursor = self.content[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.content.len());
                }
                InputAction::None
            }
            KeyCode::Home => {
                if self.multiline {
                    // Go to start of current line
                    self.cursor = self.content[..self.cursor]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                } else {
                    self.cursor = 0;
                }
                InputAction::None
            }
            KeyCode::End => {
                if self.multiline {
                    self.cursor = self.content[self.cursor..]
                        .find('\n')
                        .map(|i| self.cursor + i)
                        .unwrap_or(self.content.len());
                } else {
                    self.cursor = self.content.len();
                }
                InputAction::None
            }
            KeyCode::Enter => {
                if self.multiline
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.content.insert(self.cursor, '\n');
                    self.cursor += 1;
                    InputAction::Changed
                } else {
                    // Ctrl+Enter or Alt+Enter = submit
                    InputAction::Submit
                }
            }
            KeyCode::Esc => InputAction::Cancel,
            _ => InputAction::None,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn is_empty(&self) -> bool {
        self.content.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor = 0;
    }

    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    None,
    Changed,
    Submit,
    Cancel,
}

/// Renders the text input.
pub struct TextInputWidget<'a> {
    input: &'a TextInput,
    focused: bool,
}

impl<'a> TextInputWidget<'a> {
    pub fn new(input: &'a TextInput, focused: bool) -> Self {
        Self { input, focused }
    }
}

impl Widget for TextInputWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let text = if self.input.content.is_empty() {
            &self.input.placeholder
        } else {
            &self.input.content
        };

        let style = if self.input.content.is_empty() {
            Theme::dimmed()
        } else {
            Theme::text()
        };

        // Render text lines
        for (i, line_text) in text.split('\n').enumerate() {
            let y = area.y + i as u16;
            if y >= area.bottom() {
                break;
            }

            if self.input.content.is_empty() {
                // Placeholder
                let line = Line::from(Span::styled(line_text, style));
                buf.set_line(area.x, y, &line, area.width);
            } else {
                let line = Line::from(Span::styled(line_text, style));
                buf.set_line(area.x, y, &line, area.width);
            }
        }

        // Show cursor position indicator
        if self.focused && !self.input.content.is_empty() {
            // Find cursor position in 2D
            let before_cursor = &self.input.content[..self.input.cursor];
            let cursor_line = before_cursor.matches('\n').count();
            let cursor_col = before_cursor
                .rfind('\n')
                .map(|i| before_cursor.len() - i - 1)
                .unwrap_or(before_cursor.len());

            let cursor_y = area.y + cursor_line as u16;
            let cursor_x = area.x + cursor_col as u16;

            if cursor_y < area.bottom() && cursor_x < area.right() {
                buf[(cursor_x, cursor_y)].set_style(
                    ratatui::style::Style::default()
                        .bg(Theme::WHITE)
                        .fg(Theme::BG),
                );
            }
        }

        // Character count
        if self.focused {
            let count = self.input.char_count();
            let max = 280;
            let count_str = format!("{count}/{max}");
            let count_style = if count > max {
                Theme::like() // red
            } else if count > 260 {
                Theme::retweet() // yellow-ish (green actually, close enough)
            } else {
                Theme::dimmed()
            };

            let count_x = area.right().saturating_sub(count_str.len() as u16 + 1);
            let count_y = area.y;
            if count_y < area.bottom() {
                let line = Line::from(Span::styled(count_str, count_style));
                buf.set_line(count_x, count_y, &line, area.width);
            }
        }
    }
}
