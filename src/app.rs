use anyhow::{Context, Result};
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::time::Duration;

use crate::api::models::{TimelineResponse, Tweet};
use crate::api::XClient;
use crate::auth::AuthStore;
use crate::config::Config;
use crate::tui;
use crate::tui::theme::Theme;
use crate::tui::views::TimelineView;
use crate::tui::widgets::StatusBar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedTab {
    ForYou,
    Following,
}

impl FeedTab {
    fn label(&self) -> &'static str {
        match self {
            Self::ForYou => "For You",
            Self::Following => "Following",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Timeline,
    Loading,
    Auth,
}

pub struct App {
    client: Option<XClient>,
    account_name: String,
    tweets: Vec<Tweet>,
    selected: usize,
    scroll_offset: usize,
    tab: FeedTab,
    cursor_bottom: Option<String>,
    view: View,
    should_quit: bool,
    status_msg: Option<String>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let config = Config::load().unwrap_or_default();
        let store = AuthStore::load().unwrap_or_default();

        let (client, account_name) = match store.resolve_credentials() {
            Some(creds) => {
                let name = creds
                    .account_name
                    .clone()
                    .or_else(|| store.default.clone())
                    .unwrap_or_else(|| "user".to_string());
                match XClient::new(creds, config).await {
                    Ok(c) => (Some(c), name),
                    Err(e) => {
                        tracing::error!("Failed to initialize client: {e}");
                        (None, "?".to_string())
                    }
                }
            }
            None => (None, "not logged in".to_string()),
        };

        let view = if client.is_none() { View::Auth } else { View::Loading };
        Ok(Self {
            client,
            account_name,
            tweets: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            tab: FeedTab::Following,
            cursor_bottom: None,
            view,
            should_quit: false,
            status_msg: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = tui::init().context("Failed to initialize terminal")?;

        // Initial load
        if self.client.is_some() {
            self.view = View::Loading;
            terminal.draw(|f| self.render(f))?;
            if let Err(e) = self.fetch_timeline(false).await {
                self.status_msg = Some(format!("Error: {e}"));
                tracing::error!("Failed to fetch timeline: {e}");
            }
            self.view = View::Timeline;
        }

        // Main event loop
        while !self.should_quit {
            terminal.draw(|f| self.render(f))?;

            if let Some(key) = tui::next_key_event(Duration::from_millis(50))? {
                if tui::is_quit(&key) {
                    self.should_quit = true;
                    continue;
                }

                match self.view {
                    View::Timeline => self.handle_timeline_key(key).await,
                    View::Auth => self.handle_auth_key(key),
                    View::Loading => {} // ignore keys while loading
                }
            }
        }

        tui::restore(&mut terminal)?;
        Ok(())
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let size = frame.area();
        let layout = Layout::vertical([
            Constraint::Length(2), // tabs
            Constraint::Min(1),   // content
            Constraint::Length(1), // status bar
        ])
        .split(size);

        match self.view {
            View::Timeline => {
                self.render_tabs(frame, layout[0]);
                self.render_timeline(frame, layout[1]);
            }
            View::Loading => {
                self.render_tabs(frame, layout[0]);
                let loading = Paragraph::new("Loading timeline...")
                    .style(Theme::dimmed())
                    .block(Block::default().borders(Borders::NONE));
                frame.render_widget(loading, layout[1]);
            }
            View::Auth => {
                let msg = Paragraph::new(vec![
                    Line::from(Span::styled("Not authenticated", Theme::bold())),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Set X_AUTH_TOKEN and X_CT0 environment variables",
                        Theme::dimmed(),
                    )),
                    Line::from(Span::styled(
                        "or add credentials to ~/.config/tui-x/auth.json",
                        Theme::dimmed(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled("Press q to quit", Theme::accent())),
                ])
                .block(Block::default().borders(Borders::ALL).border_style(Theme::border()));
                frame.render_widget(msg, layout[1]);
            }
        }

        // Status bar
        let hints: &[(&str, &str)] = match self.view {
            View::Timeline => &[
                ("j/k", "nav"),
                ("l/Enter", "open"),
                ("Tab", "switch feed"),
                ("f", "like"),
                ("r", "RT"),
                ("b", "bookmark"),
                ("/", "search"),
                ("q", "quit"),
            ],
            _ => &[("q", "quit")],
        };

        let view_name = match self.view {
            View::Timeline => self.tab.label(),
            View::Loading => "Loading",
            View::Auth => "Auth",
        };

        let status = StatusBar {
            account: &self.account_name,
            view: view_name,
            hints,
        };
        frame.render_widget(status, layout[2]);
    }

    fn render_tabs(&self, frame: &mut ratatui::Frame, area: Rect) {
        let tabs = vec![FeedTab::ForYou, FeedTab::Following];

        let spans: Vec<Span> = tabs
            .iter()
            .flat_map(|tab| {
                let style = if *tab == self.tab {
                    Theme::tab_active()
                } else {
                    Theme::tab_inactive()
                };
                vec![
                    Span::styled(format!(" {} ", tab.label()), style),
                    Span::raw("  "),
                ]
            })
            .collect();

        let line = Line::from(spans);
        let tabs_widget = Paragraph::new(line)
            .block(Block::default().borders(Borders::BOTTOM).border_style(Theme::border()));
        frame.render_widget(tabs_widget, area);
    }

    fn render_timeline(&self, frame: &mut ratatui::Frame, area: Rect) {
        if self.tweets.is_empty() {
            let empty = Paragraph::new("No tweets loaded")
                .style(Theme::dimmed());
            frame.render_widget(empty, area);
            return;
        }

        let view = TimelineView::new(&self.tweets, self.selected, self.scroll_offset);
        frame.render_widget(view, area);
    }

    async fn handle_timeline_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),

            // Tab switching
            KeyCode::Tab => {
                self.tab = match self.tab {
                    FeedTab::ForYou => FeedTab::Following,
                    FeedTab::Following => FeedTab::ForYou,
                };
                self.tweets.clear();
                self.selected = 0;
                self.scroll_offset = 0;
                self.cursor_bottom = None;
                self.view = View::Loading;
                let _ = self.fetch_timeline(false).await;
                self.view = View::Timeline;
            }

            // Actions on selected tweet
            KeyCode::Char('f') => {
                if let Some(tweet) = self.tweets.get(self.selected) {
                    let id = tweet.id.clone();
                    let already_liked = tweet.favorited;
                    if let Some(ref client) = self.client {
                        let result = if already_liked {
                            client.unlike(&id).await
                        } else {
                            client.like(&id).await
                        };
                        match result {
                            Ok(_) => {
                                if let Some(t) = self.tweets.get_mut(self.selected) {
                                    t.favorited = !already_liked;
                                    if already_liked {
                                        t.like_count = t.like_count.saturating_sub(1);
                                    } else {
                                        t.like_count += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                self.status_msg = Some(format!("Like failed: {e}"));
                            }
                        }
                    }
                }
            }

            KeyCode::Char('t') => {
                if let Some(tweet) = self.tweets.get(self.selected) {
                    let id = tweet.id.clone();
                    let already_rt = tweet.retweeted;
                    if let Some(ref client) = self.client {
                        let result = if already_rt {
                            client.unretweet(&id).await
                        } else {
                            client.retweet(&id).await
                        };
                        match result {
                            Ok(_) => {
                                if let Some(t) = self.tweets.get_mut(self.selected) {
                                    t.retweeted = !already_rt;
                                    if already_rt {
                                        t.retweet_count = t.retweet_count.saturating_sub(1);
                                    } else {
                                        t.retweet_count += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                self.status_msg = Some(format!("RT failed: {e}"));
                            }
                        }
                    }
                }
            }

            KeyCode::Char('b') => {
                if let Some(tweet) = self.tweets.get(self.selected) {
                    let id = tweet.id.clone();
                    let already_bm = tweet.bookmarked;
                    if let Some(ref client) = self.client {
                        let result = if already_bm {
                            client.unbookmark(&id).await
                        } else {
                            client.bookmark(&id).await
                        };
                        match result {
                            Ok(_) => {
                                if let Some(t) = self.tweets.get_mut(self.selected) {
                                    t.bookmarked = !already_bm;
                                }
                            }
                            Err(e) => {
                                self.status_msg = Some(format!("Bookmark failed: {e}"));
                            }
                        }
                    }
                }
            }

            // Load more (when near bottom)
            KeyCode::Char(' ') => {
                if self.cursor_bottom.is_some() {
                    let _ = self.fetch_timeline(true).await;
                }
            }

            _ => {}
        }
    }

    fn handle_auth_key(&mut self, key: crossterm::event::KeyEvent) {
        // Auth view only handles quit
        if key.code == KeyCode::Char('q') {
            self.should_quit = true;
        }
    }

    fn select_next(&mut self) {
        if !self.tweets.is_empty() {
            self.selected = (self.selected + 1).min(self.tweets.len() - 1);
            // Auto-scroll
            // Simple heuristic: keep selected within visible range
            if self.selected > self.scroll_offset + 10 {
                self.scroll_offset = self.selected.saturating_sub(5);
            }
        }
    }

    fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }

    fn select_first(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
    }

    fn select_last(&mut self) {
        if !self.tweets.is_empty() {
            self.selected = self.tweets.len() - 1;
            self.scroll_offset = self.selected.saturating_sub(5);
        }
    }

    async fn fetch_timeline(&mut self, load_more: bool) -> Result<()> {
        let Some(ref client) = self.client else {
            return Ok(());
        };

        let cursor = if load_more {
            self.cursor_bottom.as_deref()
        } else {
            None
        };

        let response: TimelineResponse = match self.tab {
            FeedTab::ForYou => client.home_timeline(20, cursor).await?,
            FeedTab::Following => client.home_latest(20, cursor).await?,
        };

        if load_more {
            self.tweets.extend(response.tweets);
        } else {
            self.tweets = response.tweets;
        }
        self.cursor_bottom = response.cursor_bottom;

        Ok(())
    }
}

use ratatui::layout::Rect;
