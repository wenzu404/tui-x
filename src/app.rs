use anyhow::{Context, Result};
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::api::models::{TimelineResponse, Tweet};
use crate::api::XClient;
use crate::auth::AuthStore;
use crate::config::Config;
use crate::tui;
use crate::tui::theme::Theme;
use crate::tui::views::TimelineView;
use crate::tui::widgets::StatusBar;

// ── Messages between background tasks and the UI ────────────────────

enum ApiResult {
    Timeline {
        response: TimelineResponse,
        append: bool,
        tab: FeedTab,
    },
    ActionOk {
        action: TweetAction,
        tweet_id: String,
    },
    ActionErr {
        action: TweetAction,
        tweet_id: String,
        error: String,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy)]
enum TweetAction {
    Like,
    Unlike,
    Retweet,
    Unretweet,
    Bookmark,
    Unbookmark,
}

// ── App state ───────────────────────────────────────────────────────

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
    client: Option<Arc<XClient>>,
    account_name: String,
    tweets: Vec<Tweet>,
    selected: usize,
    scroll_offset: usize,
    tab: FeedTab,
    cursor_bottom: Option<String>,
    view: View,
    should_quit: bool,
    status_msg: Option<String>,
    // Channel for receiving API results
    api_rx: mpsc::UnboundedReceiver<ApiResult>,
    api_tx: mpsc::UnboundedSender<ApiResult>,
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
                    Ok(c) => (Some(Arc::new(c)), name),
                    Err(e) => {
                        tracing::error!("Failed to initialize client: {e}");
                        (None, "?".to_string())
                    }
                }
            }
            None => (None, "not logged in".to_string()),
        };

        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let view = if client.is_none() {
            View::Auth
        } else {
            View::Loading
        };

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
            api_rx,
            api_tx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = tui::init().context("Failed to initialize terminal")?;

        // Kick off initial timeline load (non-blocking)
        if self.client.is_some() {
            self.spawn_fetch_timeline(false);
        }

        // Main event loop — never blocks on network
        while !self.should_quit {
            // 1. Process any pending API results
            self.drain_api_results();

            // 2. Render
            terminal.draw(|f| self.render(f))?;

            // 3. Poll input (short timeout so we stay responsive)
            if let Some(key) = tui::next_key_event(Duration::from_millis(16))? {
                if tui::is_quit(&key) {
                    self.should_quit = true;
                    continue;
                }

                match self.view {
                    View::Timeline => self.handle_timeline_key(key),
                    View::Auth => self.handle_auth_key(key),
                    View::Loading => {}
                }
            }
        }

        tui::restore(&mut terminal)?;
        Ok(())
    }

    // ── Background API calls ────────────────────────────────────────

    fn spawn_fetch_timeline(&self, append: bool) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.api_tx.clone();
        let tab = self.tab;
        let cursor = if append {
            self.cursor_bottom.clone()
        } else {
            None
        };

        tokio::spawn(async move {
            let result = match tab {
                FeedTab::ForYou => client.home_timeline(20, cursor.as_deref()).await,
                FeedTab::Following => client.home_latest(20, cursor.as_deref()).await,
            };
            match result {
                Ok(response) => {
                    let _ = tx.send(ApiResult::Timeline {
                        response,
                        append,
                        tab,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("Timeline: {e}")));
                }
            }
        });
    }

    fn spawn_tweet_action(&self, tweet_id: String, action: TweetAction) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.api_tx.clone();
        let id = tweet_id.clone();

        tokio::spawn(async move {
            let result = match action {
                TweetAction::Like => client.like(&id).await,
                TweetAction::Unlike => client.unlike(&id).await,
                TweetAction::Retweet => client.retweet(&id).await,
                TweetAction::Unretweet => client.unretweet(&id).await,
                TweetAction::Bookmark => client.bookmark(&id).await,
                TweetAction::Unbookmark => client.unbookmark(&id).await,
            };
            match result {
                Ok(_) => {
                    let _ = tx.send(ApiResult::ActionOk {
                        action,
                        tweet_id,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::ActionErr {
                        action,
                        tweet_id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    /// Process all pending API results without blocking.
    fn drain_api_results(&mut self) {
        while let Ok(msg) = self.api_rx.try_recv() {
            match msg {
                ApiResult::Timeline {
                    response,
                    append,
                    tab,
                } => {
                    // Only apply if we're still on the same tab
                    if tab == self.tab {
                        if append {
                            self.tweets.extend(response.tweets);
                        } else {
                            self.tweets = response.tweets;
                            self.selected = 0;
                            self.scroll_offset = 0;
                        }
                        self.cursor_bottom = response.cursor_bottom;
                        self.view = View::Timeline;
                        self.status_msg = None;
                    }
                }
                ApiResult::ActionOk { .. } => {
                    // Already applied optimistically — nothing to do
                }
                ApiResult::ActionErr {
                    action,
                    error,
                    tweet_id,
                } => {
                    // Rollback optimistic update
                    if let Some(tweet) = self.tweets.iter_mut().find(|t| t.id == tweet_id) {
                        match action {
                            TweetAction::Like => {
                                tweet.favorited = false;
                                tweet.like_count = tweet.like_count.saturating_sub(1);
                            }
                            TweetAction::Unlike => {
                                tweet.favorited = true;
                                tweet.like_count += 1;
                            }
                            TweetAction::Retweet => {
                                tweet.retweeted = false;
                                tweet.retweet_count = tweet.retweet_count.saturating_sub(1);
                            }
                            TweetAction::Unretweet => {
                                tweet.retweeted = true;
                                tweet.retweet_count += 1;
                            }
                            TweetAction::Bookmark => {
                                tweet.bookmarked = false;
                            }
                            TweetAction::Unbookmark => {
                                tweet.bookmarked = true;
                            }
                        }
                    }
                    self.status_msg = Some(format!("{action:?} failed: {error}"));
                }
                ApiResult::Error(e) => {
                    self.status_msg = Some(e);
                    if self.view == View::Loading {
                        self.view = View::Timeline;
                    }
                }
            }
        }
    }

    // ── Rendering ───────────────────────────────────────────────────

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
                let loading = Paragraph::new("  Loading...")
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
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Theme::border()),
                );
                frame.render_widget(msg, layout[1]);
            }
        }

        // Status bar
        let hints: &[(&str, &str)] = match self.view {
            View::Timeline => &[
                ("j/k", "nav"),
                ("Tab", "feed"),
                ("f", "like"),
                ("t", "RT"),
                ("b", "mark"),
                ("Space", "more"),
                ("q", "quit"),
            ],
            _ => &[("q", "quit")],
        };

        let view_label = match self.view {
            View::Timeline => self.tab.label(),
            View::Loading => "Loading",
            View::Auth => "Auth",
        };

        // Show status message if any
        let account_display = if let Some(ref msg) = self.status_msg {
            format!("{} | {msg}", self.account_name)
        } else {
            self.account_name.clone()
        };

        let status = StatusBar {
            account: &account_display,
            view: view_label,
            hints,
        };
        frame.render_widget(status, layout[2]);
    }

    fn render_tabs(&self, frame: &mut ratatui::Frame, area: Rect) {
        let tabs = [FeedTab::ForYou, FeedTab::Following];

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
            let empty = Paragraph::new("  No tweets").style(Theme::dimmed());
            frame.render_widget(empty, area);
            return;
        }

        let view = TimelineView::new(&self.tweets, self.selected, self.scroll_offset);
        frame.render_widget(view, area);
    }

    // ── Input handling (all non-blocking now) ───────────────────────

    fn handle_timeline_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),

            // Tab switching — instant, fetch in background
            KeyCode::Tab | KeyCode::BackTab => {
                self.tab = match self.tab {
                    FeedTab::ForYou => FeedTab::Following,
                    FeedTab::Following => FeedTab::ForYou,
                };
                self.tweets.clear();
                self.selected = 0;
                self.scroll_offset = 0;
                self.cursor_bottom = None;
                self.view = View::Loading;
                self.spawn_fetch_timeline(false);
            }

            // Like (optimistic update + background confirm)
            KeyCode::Char('f') => {
                if let Some(tweet) = self.tweets.get_mut(self.selected) {
                    let id = tweet.id.clone();
                    let action = if tweet.favorited {
                        tweet.favorited = false;
                        tweet.like_count = tweet.like_count.saturating_sub(1);
                        TweetAction::Unlike
                    } else {
                        tweet.favorited = true;
                        tweet.like_count += 1;
                        TweetAction::Like
                    };
                    drop(tweet);
                    self.spawn_tweet_action(id, action);
                }
            }

            // Retweet
            KeyCode::Char('t') => {
                if let Some(tweet) = self.tweets.get_mut(self.selected) {
                    let id = tweet.id.clone();
                    let action = if tweet.retweeted {
                        tweet.retweeted = false;
                        tweet.retweet_count = tweet.retweet_count.saturating_sub(1);
                        TweetAction::Unretweet
                    } else {
                        tweet.retweeted = true;
                        tweet.retweet_count += 1;
                        TweetAction::Retweet
                    };
                    drop(tweet);
                    self.spawn_tweet_action(id, action);
                }
            }

            // Bookmark
            KeyCode::Char('b') => {
                if let Some(tweet) = self.tweets.get_mut(self.selected) {
                    let id = tweet.id.clone();
                    let action = if tweet.bookmarked {
                        tweet.bookmarked = false;
                        TweetAction::Unbookmark
                    } else {
                        tweet.bookmarked = true;
                        TweetAction::Bookmark
                    };
                    drop(tweet);
                    self.spawn_tweet_action(id, action);
                }
            }

            // Load more
            KeyCode::Char(' ') => {
                if self.cursor_bottom.is_some() {
                    self.spawn_fetch_timeline(true);
                }
            }

            _ => {}
        }
    }

    fn handle_auth_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.code == KeyCode::Char('q') {
            self.should_quit = true;
        }
    }

    // ── Selection helpers ───────────────────────────────────────────

    fn select_next(&mut self) {
        if !self.tweets.is_empty() {
            self.selected = (self.selected + 1).min(self.tweets.len() - 1);
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
}
