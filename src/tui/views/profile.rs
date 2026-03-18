use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::api::models::{Tweet, User};
use crate::tui::theme::Theme;
use crate::tui::widgets::TweetCard;

/// User profile view showing bio + their tweets.
pub struct ProfileView<'a> {
    pub user: &'a User,
    pub tweets: &'a [Tweet],
    pub selected: usize,
    pub scroll_offset: usize,
}

impl Widget for ProfileView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let mut y = area.y;

        if self.scroll_offset == 0 {
            // Profile header
            let name_line = Line::from(vec![
                Span::styled(&self.user.name, Theme::bold()),
                if self.user.verified {
                    Span::styled(" ✓", Theme::accent())
                } else {
                    Span::raw("")
                },
            ]);
            buf.set_line(area.x + 2, y, &name_line, area.width.saturating_sub(4));
            y += 1;

            if y < area.bottom() {
                let handle = Line::from(Span::styled(
                    format!("@{}", self.user.screen_name),
                    Theme::handle(),
                ));
                buf.set_line(area.x + 2, y, &handle, area.width.saturating_sub(4));
                y += 1;
            }

            // Bio
            if y < area.bottom() {
                if let Some(ref desc) = self.user.description {
                    y += 1; // blank line
                    for line_text in desc.split('\n') {
                        if y >= area.bottom() {
                            break;
                        }
                        let line = Line::from(Span::styled(line_text, Theme::text()));
                        buf.set_line(area.x + 2, y, &line, area.width.saturating_sub(4));
                        y += 1;
                    }
                }
            }

            // Stats
            if y + 1 < area.bottom() {
                y += 1;
                let stats = Line::from(vec![
                    Span::styled(compact_num(self.user.following_count), Theme::bold()),
                    Span::styled(" Following  ", Theme::dimmed()),
                    Span::styled(compact_num(self.user.followers_count), Theme::bold()),
                    Span::styled(" Followers  ", Theme::dimmed()),
                    Span::styled(compact_num(self.user.tweet_count), Theme::bold()),
                    Span::styled(" Tweets", Theme::dimmed()),
                ]);
                buf.set_line(area.x + 2, y, &stats, area.width.saturating_sub(4));
                y += 1;
            }

            // Follow status
            if y < area.bottom() {
                let follow_status = if self.user.following && self.user.followed_by {
                    "You follow each other"
                } else if self.user.following {
                    "Following"
                } else if self.user.followed_by {
                    "Follows you"
                } else {
                    ""
                };
                if !follow_status.is_empty() {
                    let line = Line::from(Span::styled(follow_status, Theme::accent()));
                    buf.set_line(area.x + 2, y, &line, area.width.saturating_sub(4));
                    y += 1;
                }
            }

            // Separator
            if y < area.bottom() {
                let sep: String = "─".repeat(area.width.saturating_sub(2) as usize);
                let line = Line::from(Span::styled(sep, Theme::border()));
                buf.set_line(area.x + 1, y, &line, area.width.saturating_sub(2));
                y += 1;
            }

            // Tweets header
            if y < area.bottom() {
                let header = Line::from(Span::styled("  Tweets", Theme::accent()));
                buf.set_line(area.x, y, &header, area.width);
                y += 1;
            }
        }

        // Tweets
        let tweet_scroll = if self.scroll_offset > 0 {
            self.scroll_offset.saturating_sub(1)
        } else {
            0
        };

        for (i, tweet) in self.tweets.iter().enumerate().skip(tweet_scroll) {
            if y >= area.bottom() {
                break;
            }
            let is_selected = i == self.selected;
            let card = TweetCard::new(tweet, is_selected, area.width);
            let h = card.height();
            let card_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: h.min(area.bottom() - y),
            };
            card.render(card_area, buf);
            y += h;
        }

        if self.tweets.is_empty() && y < area.bottom() {
            let empty = Line::from(Span::styled("  No tweets", Theme::dimmed()));
            buf.set_line(area.x, y, &empty, area.width);
        }
    }
}

fn compact_num(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
