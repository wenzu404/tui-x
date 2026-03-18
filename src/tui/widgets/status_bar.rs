use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::tui::theme::Theme;

/// Bottom status bar showing current context and keybindings.
pub struct StatusBar<'a> {
    pub account: &'a str,
    pub view: &'a str,
    pub hints: &'a [(&'a str, &'a str)], // (key, description)
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        // Background
        for x in area.x..area.right() {
            buf[(x, area.y)].set_style(Theme::accent());
        }

        let mut spans = vec![
            Span::styled(
                format!(" @{} ", self.account),
                Theme::bold().bg(Theme::BLUE).fg(Theme::BG),
            ),
            Span::styled(
                format!(" {} ", self.view),
                Theme::text().bg(Theme::DARK_GRAY),
            ),
            Span::raw(" "),
        ];

        for (key, desc) in self.hints {
            spans.push(Span::styled(
                format!(" {key} "),
                Theme::bold().bg(Theme::DARK_GRAY),
            ));
            spans.push(Span::styled(format!(" {desc} "), Theme::dimmed()));
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}
