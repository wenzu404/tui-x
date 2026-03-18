use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::api::models::Tweet;
use crate::tui::widgets::TweetCard;

/// State for a scrollable timeline view.
pub struct TimelineView<'a> {
    tweets: &'a [Tweet],
    selected: usize,
    scroll_offset: usize,
}

impl<'a> TimelineView<'a> {
    pub fn new(tweets: &'a [Tweet], selected: usize, scroll_offset: usize) -> Self {
        Self {
            tweets,
            selected,
            scroll_offset,
        }
    }
}

impl Widget for TimelineView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.tweets.is_empty() || area.height == 0 {
            return;
        }

        let mut y = area.y;

        for (i, tweet) in self.tweets.iter().enumerate().skip(self.scroll_offset) {
            if y >= area.bottom() {
                break;
            }

            let is_selected = i == self.selected;
            let card = TweetCard::new(tweet, is_selected, area.width);
            let card_height = card.height();

            let card_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: card_height.min(area.bottom() - y),
            };

            card.render(card_area, buf);
            y += card_height;
        }
    }
}
