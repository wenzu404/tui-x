use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::api::models::Tweet;
use crate::tui::theme::Theme;
use crate::tui::widgets::TweetCard;

/// Full tweet detail view with parent context and replies.
pub struct TweetDetailView<'a> {
    pub main_tweet: &'a Tweet,
    pub parents: &'a [Tweet],
    pub replies: &'a [Tweet],
    pub selected_reply: usize,
    pub scroll_offset: usize,
}

impl Widget for TweetDetailView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        let mut y = area.y;

        // Skip based on scroll offset
        let mut item_index: usize = 0;
        let total_before_replies = self.parents.len() + 1; // parents + main

        // Render parent tweets (dimmed, compact)
        for parent in self.parents.iter() {
            if item_index < self.scroll_offset {
                item_index += 1;
                continue;
            }
            if y >= area.bottom() {
                break;
            }

            let card = TweetCard::new(parent, false, area.width);
            let h = card.height();
            let card_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: h.min(area.bottom() - y),
            };
            card.render(card_area, buf);

            // Draw reply thread connector
            if y + h < area.bottom() {
                let connector = Line::from(Span::styled("  │", Theme::dimmed()));
                buf.set_line(area.x, y + h.saturating_sub(1), &connector, area.width);
            }

            y += h;
            item_index += 1;
        }

        // Render main tweet (highlighted)
        if item_index >= self.scroll_offset && y < area.bottom() {
            // Header
            let header = Line::from(Span::styled("── Tweet ──", Theme::accent()));
            buf.set_line(area.x + 1, y, &header, area.width.saturating_sub(2));
            y += 1;

            if y < area.bottom() {
                let card = TweetCard::new(self.main_tweet, true, area.width);
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
        }
        item_index += 1;

        // Replies header
        if y < area.bottom() && !self.replies.is_empty() {
            let replies_header = Line::from(Span::styled(
                format!("── Replies ({}) ──", self.replies.len()),
                Theme::accent(),
            ));
            buf.set_line(area.x + 1, y, &replies_header, area.width.saturating_sub(2));
            y += 1;
        }

        // Render replies
        for (i, reply) in self.replies.iter().enumerate() {
            if item_index < self.scroll_offset {
                item_index += 1;
                continue;
            }
            if y >= area.bottom() {
                break;
            }

            let is_selected = i == self.selected_reply;
            let card = TweetCard::new(reply, is_selected, area.width);
            let h = card.height();
            let card_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: h.min(area.bottom() - y),
            };
            card.render(card_area, buf);
            y += h;
            item_index += 1;
        }
    }
}
