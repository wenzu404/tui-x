use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::api::models::Tweet;
use crate::tui::theme::Theme;

/// Renders a single tweet as a card.
pub struct TweetCard<'a> {
    tweet: &'a Tweet,
    selected: bool,
    width: u16,
}

impl<'a> TweetCard<'a> {
    pub fn new(tweet: &'a Tweet, selected: bool, width: u16) -> Self {
        Self {
            tweet,
            selected,
            width,
        }
    }

    /// Calculate the height needed to render this tweet.
    pub fn height(&self) -> u16 {
        let text_lines = textwrap_lines(&self.tweet.text, self.width.saturating_sub(4) as usize);
        let header_lines = 1u16;
        let text_height = text_lines.len().max(1) as u16;
        let stats_lines = 1u16;
        let separator = 1u16;
        let retweet_banner = if self.tweet.is_retweet { 1 } else { 0 };

        retweet_banner + header_lines + text_height + stats_lines + separator
    }
}

impl Widget for TweetCard<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        let base_style = if self.selected {
            Theme::selected()
        } else {
            Theme::text()
        };

        // Fill background if selected
        if self.selected {
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    buf[(x, y)].set_style(Theme::selected());
                }
            }
        }

        let mut y = area.y;
        let x = area.x + 2; // left padding
        let max_w = area.width.saturating_sub(4) as usize;

        // Retweet banner
        if self.tweet.is_retweet {
            if let Some(ref by) = self.tweet.retweeted_by {
                let line = Line::from(vec![
                    Span::styled("↻ ", Theme::retweet()),
                    Span::styled(format!("{by} retweeted"), Theme::dimmed()),
                ]);
                buf.set_line(x, y, &line, area.width.saturating_sub(4));
                y += 1;
            }
        }

        if y >= area.bottom() {
            return;
        }

        // Header: display name · @handle · time
        let time_ago = self
            .tweet
            .created_at
            .map(|t| format_time_ago(t))
            .unwrap_or_default();

        let header = Line::from(vec![
            Span::styled(&self.tweet.author.name, Theme::bold()),
            Span::styled(" @", Theme::handle()),
            Span::styled(&self.tweet.author.screen_name, Theme::handle()),
            Span::styled(format!(" · {time_ago}"), Theme::dimmed()),
        ]);
        buf.set_line(x, y, &header, area.width.saturating_sub(4));
        y += 1;

        if y >= area.bottom() {
            return;
        }

        // Tweet text (word-wrapped)
        let text_lines = textwrap_lines(&self.tweet.text, max_w);
        for line_text in &text_lines {
            if y >= area.bottom() {
                break;
            }
            let line = Line::from(Span::styled(line_text.as_str(), base_style));
            buf.set_line(x, y, &line, area.width.saturating_sub(4));
            y += 1;
        }

        if y >= area.bottom() {
            return;
        }

        // Stats line: replies · retweets · likes · views
        let like_style = if self.tweet.favorited {
            Theme::like()
        } else {
            Theme::dimmed()
        };
        let rt_style = if self.tweet.retweeted {
            Theme::retweet()
        } else {
            Theme::dimmed()
        };

        let stats = Line::from(vec![
            Span::styled(format!("💬 {} ", compact_num(self.tweet.reply_count)), Theme::dimmed()),
            Span::styled(format!("↻ {} ", compact_num(self.tweet.retweet_count)), rt_style),
            Span::styled(format!("♥ {} ", compact_num(self.tweet.like_count)), like_style),
            Span::styled(
                format!(
                    "👁 {}",
                    self.tweet
                        .view_count
                        .map(compact_num)
                        .unwrap_or_default()
                ),
                Theme::dimmed(),
            ),
        ]);
        buf.set_line(x, y, &stats, area.width.saturating_sub(4));
        y += 1;

        // Separator line
        if y < area.bottom() {
            let sep: String = "─".repeat(area.width.saturating_sub(2) as usize);
            let line = Line::from(Span::styled(sep, Theme::border()));
            buf.set_line(area.x + 1, y, &line, area.width.saturating_sub(2));
        }
    }
}

/// Simple word-wrap implementation.
fn textwrap_lines(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current_line = String::new();
        for word in paragraph.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Format a timestamp as a relative time string.
fn format_time_ago(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now - time;

    if diff.num_seconds() < 60 {
        format!("{}s", diff.num_seconds())
    } else if diff.num_minutes() < 60 {
        format!("{}m", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h", diff.num_hours())
    } else if diff.num_days() < 7 {
        format!("{}d", diff.num_days())
    } else {
        time.format("%b %d").to_string()
    }
}

/// Compact number formatting: 1234 -> "1.2K", 1234567 -> "1.2M".
fn compact_num(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
