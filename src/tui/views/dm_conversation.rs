use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use crate::api::models::DmMessage;
use crate::tui::theme::Theme;
use crate::tui::widgets::{TextInput, TextInputWidget};

/// A single DM conversation view with messages and input.
pub struct DmConversationView<'a> {
    pub participant_name: &'a str,
    pub messages: &'a [DmMessage],
    pub my_user_id: &'a str,
    pub input: &'a TextInput,
    pub scroll_offset: usize,
}

impl Widget for DmConversationView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 4 {
            return;
        }

        let layout = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(3),   // messages
            Constraint::Length(3), // input box
        ])
        .split(area);

        // Header
        let header = Line::from(vec![
            Span::styled("  ", Theme::text()),
            Span::styled(self.participant_name, Theme::bold()),
        ]);
        buf.set_line(layout[0].x, layout[0].y, &header, layout[0].width);

        // Messages (newest at bottom)
        let msg_area = layout[1];
        let visible_lines = msg_area.height as usize;
        let start = self
            .messages
            .len()
            .saturating_sub(visible_lines + self.scroll_offset);
        let end = self.messages.len().saturating_sub(self.scroll_offset);

        let mut y = msg_area.y;
        for msg in &self.messages[start..end] {
            if y >= msg_area.bottom() {
                break;
            }

            let is_mine = msg.sender_id == self.my_user_id;
            let x = if is_mine {
                // Right-align my messages
                msg_area.x + 4
            } else {
                msg_area.x + 2
            };

            let style = if is_mine {
                Theme::accent()
            } else {
                Theme::text()
            };

            let max_w = msg_area.width.saturating_sub(6) as usize;
            for line_text in msg.text.split('\n') {
                if y >= msg_area.bottom() {
                    break;
                }
                let display: String = line_text.chars().take(max_w).collect();
                let line = Line::from(Span::styled(display, style));
                buf.set_line(x, y, &line, msg_area.width.saturating_sub(6));
                y += 1;
            }

            // Timestamp
            if y < msg_area.bottom() {
                let time = msg
                    .created_at
                    .map(|t| t.format("%H:%M").to_string())
                    .unwrap_or_default();
                let time_line = Line::from(Span::styled(time, Theme::dimmed()));
                buf.set_line(x, y, &time_line, msg_area.width.saturating_sub(6));
                y += 1;
            }
        }

        // Input box
        let input_block = Block::default()
            .title(Span::styled(" Message ", Theme::dimmed()))
            .borders(Borders::ALL)
            .border_style(Theme::border());
        let input_inner = input_block.inner(layout[2]);
        input_block.render(layout[2], buf);

        let input_widget = TextInputWidget::new(self.input, true);
        input_widget.render(input_inner, buf);
    }
}
