use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use crate::tui::theme::Theme;
use crate::tui::widgets::{TextInput, TextInputWidget};

#[derive(Debug, Clone)]
pub enum ComposeMode {
    NewTweet,
    Reply {
        tweet_id: String,
        reply_to_user: String,
    },
    Quote {
        tweet_url: String,
    },
}

impl ComposeMode {
    pub fn title(&self) -> &str {
        match self {
            Self::NewTweet => "New Tweet",
            Self::Reply { .. } => "Reply",
            Self::Quote { .. } => "Quote Tweet",
        }
    }
}

/// Compose tweet view.
pub struct ComposeView<'a> {
    pub input: &'a TextInput,
    pub mode: &'a ComposeMode,
}

impl Widget for ComposeView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 5 {
            return;
        }

        // Center the compose box
        let box_width = area.width.min(80);
        let box_x = area.x + (area.width.saturating_sub(box_width)) / 2;
        let box_height = area.height.min(20);
        let box_y = area.y + 1;

        let compose_area = Rect {
            x: box_x,
            y: box_y,
            width: box_width,
            height: box_height,
        };

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.mode.title()),
                Theme::accent(),
            ))
            .borders(Borders::ALL)
            .border_style(Theme::accent());
        let inner = block.inner(compose_area);
        block.render(compose_area, buf);

        let layout = Layout::vertical([
            Constraint::Length(2), // context line
            Constraint::Min(3),   // text input
            Constraint::Length(1), // hints
        ])
        .split(inner);

        // Context line (reply-to info or quote info)
        match self.mode {
            ComposeMode::Reply { reply_to_user, .. } => {
                let ctx = Line::from(vec![
                    Span::styled("  Replying to ", Theme::dimmed()),
                    Span::styled(format!("@{reply_to_user}"), Theme::accent()),
                ]);
                buf.set_line(layout[0].x, layout[0].y, &ctx, layout[0].width);
            }
            ComposeMode::Quote { tweet_url } => {
                let display_url: String = tweet_url.chars().take(50).collect();
                let ctx = Line::from(vec![
                    Span::styled("  Quoting ", Theme::dimmed()),
                    Span::styled(display_url, Theme::accent()),
                ]);
                buf.set_line(layout[0].x, layout[0].y, &ctx, layout[0].width);
            }
            ComposeMode::NewTweet => {}
        }

        // Text input
        let input_area = Rect {
            x: layout[1].x + 1,
            y: layout[1].y,
            width: layout[1].width.saturating_sub(2),
            height: layout[1].height,
        };
        let input_widget = TextInputWidget::new(self.input, true);
        input_widget.render(input_area, buf);

        // Hints
        let hints = Line::from(vec![
            Span::styled(" Ctrl+Enter ", Theme::bold().bg(Theme::DARK_GRAY)),
            Span::styled(" send  ", Theme::dimmed()),
            Span::styled(" Esc ", Theme::bold().bg(Theme::DARK_GRAY)),
            Span::styled(" cancel", Theme::dimmed()),
        ]);
        buf.set_line(layout[2].x, layout[2].y, &hints, layout[2].width);
    }
}
