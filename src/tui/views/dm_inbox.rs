use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::api::models::DmConversation;
use crate::tui::theme::Theme;

/// DM inbox listing conversations.
pub struct DmInboxView<'a> {
    pub conversations: &'a [DmConversation],
    pub selected: usize,
}

impl Widget for DmInboxView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        if self.conversations.is_empty() {
            let empty = Line::from(Span::styled("  No messages", Theme::dimmed()));
            buf.set_line(area.x, area.y, &empty, area.width);
            return;
        }

        let mut y = area.y;

        // Header
        let header = Line::from(Span::styled("  Messages", Theme::bold()));
        buf.set_line(area.x, y, &header, area.width);
        y += 2;

        for (i, convo) in self.conversations.iter().enumerate() {
            if y + 2 >= area.bottom() {
                break;
            }

            let is_selected = i == self.selected;
            let x = area.x + 2;
            let max_w = area.width.saturating_sub(4);

            // Background for selected
            if is_selected {
                for row in y..y + 3 {
                    if row < area.bottom() {
                        for col in area.x..area.right() {
                            buf[(col, row)].set_style(Theme::selected());
                        }
                    }
                }
            }

            // Name + unread indicator
            let name_style = if convo.unread {
                Theme::bold()
            } else {
                Theme::text()
            };
            let unread_dot = if convo.unread { "● " } else { "  " };

            let name_line = Line::from(vec![
                Span::styled(unread_dot, Theme::accent()),
                Span::styled(&convo.participant.name, name_style),
                Span::styled(
                    format!(" @{}", convo.participant.screen_name),
                    Theme::handle(),
                ),
            ]);
            buf.set_line(x, y, &name_line, max_w);
            y += 1;

            // Last message preview
            if let Some(ref msg) = convo.last_message {
                let preview: String = msg.text.chars().take(max_w as usize).collect();
                let preview = preview.replace('\n', " ");
                let line = Line::from(Span::styled(preview, Theme::dimmed()));
                buf.set_line(x + 2, y, &line, max_w.saturating_sub(2));
            }
            y += 1;

            // Separator
            if y < area.bottom() {
                let sep: String = "─".repeat(max_w as usize);
                let line = Line::from(Span::styled(sep, Theme::border()));
                buf.set_line(x, y, &line, max_w);
                y += 1;
            }
        }
    }
}
