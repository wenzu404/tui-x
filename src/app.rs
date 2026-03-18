use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::api::models::*;
use crate::api::XClient;
use crate::auth::AuthStore;
use crate::config::Config;
use crate::tui;
use crate::tui::theme::Theme;
use crate::tui::views::*;
use crate::tui::widgets::*;

// ── Async messages ──────────────────────────────────────────────────

enum ApiResult {
    Timeline {
        response: TimelineResponse,
        append: bool,
        tab: FeedTab,
    },
    TweetDetail(Box<ThreadResponse>),
    UserProfile {
        user: User,
    },
    UserTweets {
        response: TimelineResponse,
        user_id: String,
        append: bool,
    },
    DmInbox(ParsedDmInbox),
    DmMessages {
        conversation_id: String,
        messages: Vec<DmMessage>,
    },
    TweetSent,
    DmSent,
    ActionOk,
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

// ── View stack ──────────────────────────────────────────────────────

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

/// Each screen the user can navigate to.
enum Screen {
    Timeline {
        tab: FeedTab,
        tweets: Vec<Tweet>,
        selected: usize,
        scroll_offset: usize,
        cursor_bottom: Option<String>,
    },
    TweetDetail {
        tweet_id: String,
        main_tweet: Option<Tweet>,
        parents: Vec<Tweet>,
        replies: Vec<Tweet>,
        selected_reply: usize,
        scroll_offset: usize,
    },
    Profile {
        screen_name: String,
        user: Option<User>,
        tweets: Vec<Tweet>,
        selected: usize,
        scroll_offset: usize,
        cursor_bottom: Option<String>,
    },
    Compose {
        mode: ComposeMode,
        input: TextInput,
    },
    DmInbox {
        conversations: Vec<DmConversation>,
        selected: usize,
    },
    DmConversation {
        conversation_id: String,
        participant_name: String,
        messages: Vec<DmMessage>,
        input: TextInput,
        scroll_offset: usize,
    },
    Loading {
        message: String,
    },
    Auth,
}

pub struct App {
    client: Option<Arc<XClient>>,
    account_name: String,
    my_user_id: String,
    screens: Vec<Screen>,
    should_quit: bool,
    status_msg: Option<String>,
    api_rx: mpsc::UnboundedReceiver<ApiResult>,
    api_tx: mpsc::UnboundedSender<ApiResult>,
    /// Cached DM messages by conversation_id.
    dm_messages: std::collections::HashMap<String, Vec<DmMessage>>,
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

        let initial_screen = if client.is_some() {
            Screen::Loading {
                message: "Loading timeline...".to_string(),
            }
        } else {
            Screen::Auth
        };

        let mut app = App {
            client,
            account_name,
            my_user_id: String::new(),
            screens: vec![initial_screen],
            should_quit: false,
            status_msg: None,
            api_rx,
            api_tx,
            dm_messages: std::collections::HashMap::new(),
        };

        // Start initial load
        if app.client.is_some() {
            app.spawn_fetch_timeline(FeedTab::Following, false, None);
        }

        Ok(app)
    }

    fn current_screen(&self) -> &Screen {
        self.screens.last().unwrap()
    }

    fn current_screen_mut(&mut self) -> &mut Screen {
        self.screens.last_mut().unwrap()
    }

    fn push_screen(&mut self, screen: Screen) {
        self.screens.push(screen);
    }

    fn pop_screen(&mut self) {
        if self.screens.len() > 1 {
            self.screens.pop();
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = tui::init().context("Failed to initialize terminal")?;

        while !self.should_quit {
            self.drain_api_results();
            terminal.draw(|f| self.render(f))?;

            if let Some(key) = tui::next_key_event(Duration::from_millis(16))? {
                // Global quit
                if tui::is_quit(&key) && !self.is_input_mode() {
                    self.should_quit = true;
                    continue;
                }
                self.handle_key(key);
            }
        }

        tui::restore(&mut terminal)?;
        Ok(())
    }

    fn is_input_mode(&self) -> bool {
        matches!(
            self.current_screen(),
            Screen::Compose { .. } | Screen::DmConversation { .. }
        )
    }

    // ── Background spawners ─────────────────────────────────────────

    fn spawn_fetch_timeline(
        &self,
        tab: FeedTab,
        append: bool,
        cursor: Option<String>,
    ) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
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
                    let _ = tx.send(ApiResult::Error(e.to_string()));
                }
            }
        });
    }

    fn spawn_tweet_detail(&self, tweet_id: String) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            match client.tweet_detail(&tweet_id).await {
                Ok(thread) => {
                    let _ = tx.send(ApiResult::TweetDetail(Box::new(thread)));
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("Thread: {e}")));
                }
            }
        });
    }

    fn spawn_user_profile(&self, screen_name: String) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        let sn = screen_name.clone();
        tokio::spawn(async move {
            match client.user_by_screen_name(&sn).await {
                Ok(user) => {
                    let user_id = user.id.clone();
                    let _ = tx.send(ApiResult::UserProfile { user });
                    // Also fetch their tweets
                    match client.user_tweets(&user_id, 20, None).await {
                        Ok(response) => {
                            let _ = tx.send(ApiResult::UserTweets {
                                response,
                                user_id,
                                append: false,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(ApiResult::Error(format!("User tweets: {e}")));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("Profile: {e}")));
                }
            }
        });
    }

    fn spawn_dm_inbox(&self) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            match client.dm_inbox().await {
                Ok(data) => {
                    let parsed = parse_dm_inbox(&data);
                    let _ = tx.send(ApiResult::DmInbox(parsed));
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("DMs: {e}")));
                }
            }
        });
    }

    fn spawn_send_tweet(&self, text: String, mode: ComposeMode) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let result = match &mode {
                ComposeMode::NewTweet => {
                    client.create_tweet(&text, None, None, vec![]).await
                }
                ComposeMode::Reply { tweet_id, .. } => {
                    client
                        .create_tweet(&text, Some(tweet_id), None, vec![])
                        .await
                }
                ComposeMode::Quote { tweet_url } => {
                    client
                        .create_tweet(&text, None, Some(tweet_url), vec![])
                        .await
                }
            };
            match result {
                Ok(_) => {
                    let _ = tx.send(ApiResult::TweetSent);
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("Tweet: {e}")));
                }
            }
        });
    }

    fn spawn_send_dm(&self, conversation_id: String, text: String) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            match client.send_dm(&conversation_id, &text).await {
                Ok(_) => {
                    let _ = tx.send(ApiResult::DmSent);
                }
                Err(e) => {
                    let _ = tx.send(ApiResult::Error(format!("DM: {e}")));
                }
            }
        });
    }

    fn spawn_tweet_action(&self, tweet_id: String, action: TweetAction) {
        let Some(client) = self.client.clone() else { return };
        let tx = self.api_tx.clone();
        tokio::spawn(async move {
            let result = match action {
                TweetAction::Like => client.like(&tweet_id).await,
                TweetAction::Unlike => client.unlike(&tweet_id).await,
                TweetAction::Retweet => client.retweet(&tweet_id).await,
                TweetAction::Unretweet => client.unretweet(&tweet_id).await,
                TweetAction::Bookmark => client.bookmark(&tweet_id).await,
                TweetAction::Unbookmark => client.unbookmark(&tweet_id).await,
            };
            match result {
                Ok(_) => {
                    let _ = tx.send(ApiResult::ActionOk);
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

    // ── Process API results ─────────────────────────────────────────

    fn drain_api_results(&mut self) {
        while let Ok(msg) = self.api_rx.try_recv() {
            match msg {
                ApiResult::Timeline {
                    response,
                    append,
                    tab,
                } => {
                    // Find or create the right timeline screen
                    let screen = self.current_screen_mut();
                    match screen {
                        Screen::Loading { .. } => {
                            *screen = Screen::Timeline {
                                tab,
                                tweets: response.tweets,
                                selected: 0,
                                scroll_offset: 0,
                                cursor_bottom: response.cursor_bottom,
                            };
                        }
                        Screen::Timeline {
                            tab: existing_tab,
                            tweets,
                            cursor_bottom,
                            selected,
                            scroll_offset,
                            ..
                        } if *existing_tab == tab => {
                            if append {
                                tweets.extend(response.tweets);
                            } else {
                                *tweets = response.tweets;
                                *selected = 0;
                                *scroll_offset = 0;
                            }
                            *cursor_bottom = response.cursor_bottom;
                        }
                        _ => {}
                    }
                }
                ApiResult::TweetDetail(thread) => {
                    let screen = self.current_screen_mut();
                    if let Screen::TweetDetail {
                        main_tweet,
                        parents,
                        replies,
                        ..
                    } = screen
                    {
                        *main_tweet = Some(thread.main_tweet);
                        *parents = thread.parents;
                        *replies = thread.replies;
                    }
                }
                ApiResult::UserProfile { user } => {
                    let uid = user.id.clone();
                    let screen = self.current_screen_mut();
                    if let Screen::Profile {
                        user: existing_user,
                        ..
                    } = screen
                    {
                        *existing_user = Some(user);
                    }
                    self.my_user_id = uid;
                }
                ApiResult::UserTweets {
                    response, append, ..
                } => {
                    let screen = self.current_screen_mut();
                    if let Screen::Profile {
                        tweets,
                        cursor_bottom,
                        ..
                    } = screen
                    {
                        if append {
                            tweets.extend(response.tweets);
                        } else {
                            *tweets = response.tweets;
                        }
                        *cursor_bottom = response.cursor_bottom;
                    }
                }
                ApiResult::DmInbox(parsed) => {
                    if parsed.my_user_id.is_some() && self.my_user_id.is_empty() {
                        self.my_user_id = parsed.my_user_id.unwrap_or_default();
                    }
                    self.dm_messages = parsed.messages;
                    let screen = self.current_screen_mut();
                    match screen {
                        Screen::Loading { .. } | Screen::DmInbox { .. } => {
                            *screen = Screen::DmInbox {
                                conversations: parsed.conversations,
                                selected: 0,
                            };
                        }
                        _ => {}
                    }
                }
                ApiResult::DmMessages {
                    messages,
                    conversation_id: _,
                } => {
                    let screen = self.current_screen_mut();
                    if let Screen::DmConversation {
                        messages: existing, ..
                    } = screen
                    {
                        *existing = messages;
                    }
                }
                ApiResult::TweetSent => {
                    self.status_msg = Some("Tweet sent!".to_string());
                    self.pop_screen();
                }
                ApiResult::DmSent => {
                    self.status_msg = Some("DM sent!".to_string());
                    // Refresh DM inbox
                    self.spawn_dm_inbox();
                }
                ApiResult::ActionOk => {}
                ApiResult::ActionErr {
                    action,
                    tweet_id,
                    error,
                } => {
                    // Rollback optimistic update
                    self.rollback_action(&tweet_id, action);
                    self.status_msg = Some(format!("{action:?} failed: {error}"));
                }
                ApiResult::Error(e) => {
                    self.status_msg = Some(e);
                    // If we're on a loading screen, go back
                    if matches!(self.current_screen(), Screen::Loading { .. }) {
                        if self.screens.len() > 1 {
                            self.pop_screen();
                        }
                    }
                }
            }
        }
    }

    fn rollback_action(&mut self, tweet_id: &str, action: TweetAction) {
        // Find the tweet in current screen's tweets
        let tweets = match self.current_screen_mut() {
            Screen::Timeline { tweets, .. } => tweets,
            Screen::TweetDetail { replies, .. } => replies,
            Screen::Profile { tweets, .. } => tweets,
            _ => return,
        };
        if let Some(tweet) = tweets.iter_mut().find(|t| t.id == tweet_id) {
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
                TweetAction::Bookmark => tweet.bookmarked = false,
                TweetAction::Unbookmark => tweet.bookmarked = true,
            }
        }
    }

    // ── Rendering ───────────────────────────────────────────────────

    fn render(&self, frame: &mut ratatui::Frame) {
        let size = frame.area();
        let layout = Layout::vertical([
            Constraint::Min(1),   // content
            Constraint::Length(1), // status bar
        ])
        .split(size);

        match self.current_screen() {
            Screen::Timeline {
                tab,
                tweets,
                selected,
                scroll_offset,
                ..
            } => {
                let content_layout = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Min(1),
                ])
                .split(layout[0]);

                self.render_tabs(frame, content_layout[0], *tab);

                if tweets.is_empty() {
                    let empty = Paragraph::new("  No tweets").style(Theme::dimmed());
                    frame.render_widget(empty, content_layout[1]);
                } else {
                    let view = TimelineView::new(tweets, *selected, *scroll_offset);
                    frame.render_widget(view, content_layout[1]);
                }
            }
            Screen::TweetDetail {
                main_tweet,
                parents,
                replies,
                selected_reply,
                scroll_offset,
                ..
            } => {
                if let Some(tweet) = main_tweet {
                    let view = TweetDetailView {
                        main_tweet: tweet,
                        parents,
                        replies,
                        selected_reply: *selected_reply,
                        scroll_offset: *scroll_offset,
                    };
                    frame.render_widget(view, layout[0]);
                } else {
                    let loading = Paragraph::new("  Loading thread...").style(Theme::dimmed());
                    frame.render_widget(loading, layout[0]);
                }
            }
            Screen::Profile {
                user,
                tweets,
                selected,
                scroll_offset,
                ..
            } => {
                if let Some(u) = user {
                    let view = ProfileView {
                        user: u,
                        tweets,
                        selected: *selected,
                        scroll_offset: *scroll_offset,
                    };
                    frame.render_widget(view, layout[0]);
                } else {
                    let loading = Paragraph::new("  Loading profile...").style(Theme::dimmed());
                    frame.render_widget(loading, layout[0]);
                }
            }
            Screen::Compose { mode, input } => {
                let view = ComposeView { input, mode };
                frame.render_widget(view, layout[0]);
            }
            Screen::DmInbox {
                conversations,
                selected,
            } => {
                let view = DmInboxView {
                    conversations,
                    selected: *selected,
                };
                frame.render_widget(view, layout[0]);
            }
            Screen::DmConversation {
                participant_name,
                messages,
                input,
                scroll_offset,
                ..
            } => {
                let view = DmConversationView {
                    participant_name,
                    messages,
                    my_user_id: &self.my_user_id,
                    input,
                    scroll_offset: *scroll_offset,
                };
                frame.render_widget(view, layout[0]);
            }
            Screen::Loading { message } => {
                let loading = Paragraph::new(format!("  {message}")).style(Theme::dimmed());
                frame.render_widget(loading, layout[0]);
            }
            Screen::Auth => {
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
                frame.render_widget(msg, layout[0]);
            }
        }

        // Status bar
        let (hints, view_label) = self.current_hints();
        let account_display = if let Some(ref msg) = self.status_msg {
            format!("{} | {msg}", self.account_name)
        } else {
            self.account_name.clone()
        };
        let status = StatusBar {
            account: &account_display,
            view: view_label,
            hints: &hints,
        };
        frame.render_widget(status, layout[1]);
    }

    fn render_tabs(&self, frame: &mut ratatui::Frame, area: Rect, active_tab: FeedTab) {
        let tabs = [FeedTab::ForYou, FeedTab::Following];
        let spans: Vec<Span> = tabs
            .iter()
            .flat_map(|tab| {
                let style = if *tab == active_tab {
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
        let tabs_widget = Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::BOTTOM).border_style(Theme::border()));
        frame.render_widget(tabs_widget, area);
    }

    fn current_hints(&self) -> (Vec<(&str, &str)>, &str) {
        match self.current_screen() {
            Screen::Timeline { tab, .. } => (
                vec![
                    ("j/k", "nav"),
                    ("Enter", "open"),
                    ("Tab", "feed"),
                    ("f", "like"),
                    ("t", "RT"),
                    ("b", "mark"),
                    ("n", "tweet"),
                    ("p", "profile"),
                    ("d", "DMs"),
                    ("Space", "more"),
                ],
                tab.label(),
            ),
            Screen::TweetDetail { .. } => (
                vec![
                    ("j/k", "nav"),
                    ("r", "reply"),
                    ("f", "like"),
                    ("q", "quote"),
                    ("Esc", "back"),
                ],
                "Thread",
            ),
            Screen::Profile { .. } => (
                vec![
                    ("j/k", "nav"),
                    ("Enter", "open"),
                    ("Space", "more"),
                    ("Esc", "back"),
                ],
                "Profile",
            ),
            Screen::Compose { .. } => (
                vec![("C-Enter", "send"), ("Esc", "cancel")],
                "Compose",
            ),
            Screen::DmInbox { .. } => (
                vec![
                    ("j/k", "nav"),
                    ("Enter", "open"),
                    ("Esc", "back"),
                ],
                "Messages",
            ),
            Screen::DmConversation { .. } => (
                vec![("Enter", "send"), ("Esc", "back")],
                "DM",
            ),
            Screen::Loading { .. } => (vec![], "Loading"),
            Screen::Auth => (vec![("q", "quit")], "Auth"),
        }
    }

    // ── Input handling ──────────────────────────────────────────────

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        // Clear status message on any key
        self.status_msg = None;

        match self.current_screen() {
            Screen::Timeline { .. } => self.handle_timeline_key(key),
            Screen::TweetDetail { .. } => self.handle_detail_key(key),
            Screen::Profile { .. } => self.handle_profile_key(key),
            Screen::Compose { .. } => self.handle_compose_key(key),
            Screen::DmInbox { .. } => self.handle_dm_inbox_key(key),
            Screen::DmConversation { .. } => self.handle_dm_conversation_key(key),
            Screen::Loading { .. } => {
                if key.code == KeyCode::Esc {
                    self.pop_screen();
                }
            }
            Screen::Auth => {
                if key.code == KeyCode::Char('q') {
                    self.should_quit = true;
                }
            }
        }
    }

    fn handle_timeline_key(&mut self, key: crossterm::event::KeyEvent) {
        // First extract data we need from the current screen
        let (tab, tweet_count, selected, cursor_bottom) = {
            let Screen::Timeline {
                tab,
                tweets,
                selected,
                cursor_bottom,
                ..
            } = self.current_screen()
            else {
                return;
            };
            (*tab, tweets.len(), *selected, cursor_bottom.clone())
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Screen::Timeline {
                    tweets,
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    if !tweets.is_empty() {
                        *selected = (*selected + 1).min(tweets.len() - 1);
                        if *selected > *scroll_offset + 10 {
                            *scroll_offset = selected.saturating_sub(5);
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Screen::Timeline {
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    *selected = selected.saturating_sub(1);
                    if *selected < *scroll_offset {
                        *scroll_offset = *selected;
                    }
                }
            }
            KeyCode::Char('g') => {
                if let Screen::Timeline {
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    *selected = 0;
                    *scroll_offset = 0;
                }
            }
            KeyCode::Char('G') => {
                if let Screen::Timeline {
                    tweets,
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    if !tweets.is_empty() {
                        *selected = tweets.len() - 1;
                        *scroll_offset = selected.saturating_sub(5);
                    }
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let new_tab = match tab {
                    FeedTab::ForYou => FeedTab::Following,
                    FeedTab::Following => FeedTab::ForYou,
                };
                if let Screen::Timeline {
                    tab,
                    tweets,
                    selected,
                    scroll_offset,
                    cursor_bottom,
                } = self.current_screen_mut()
                {
                    *tab = new_tab;
                    tweets.clear();
                    *selected = 0;
                    *scroll_offset = 0;
                    *cursor_bottom = None;
                }
                self.spawn_fetch_timeline(new_tab, false, None);
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                // Open tweet detail
                if let Screen::Timeline { tweets, selected, .. } = self.current_screen() {
                    if let Some(tweet) = tweets.get(*selected) {
                        let tweet_id = tweet.id.clone();
                        self.push_screen(Screen::TweetDetail {
                            tweet_id: tweet_id.clone(),
                            main_tweet: None,
                            parents: Vec::new(),
                            replies: Vec::new(),
                            selected_reply: 0,
                            scroll_offset: 0,
                        });
                        self.spawn_tweet_detail(tweet_id);
                    }
                }
            }
            KeyCode::Char('p') => {
                // Open profile of selected tweet's author
                if let Screen::Timeline { tweets, selected, .. } = self.current_screen() {
                    if let Some(tweet) = tweets.get(*selected) {
                        let sn = tweet.author.screen_name.clone();
                        self.push_screen(Screen::Profile {
                            screen_name: sn.clone(),
                            user: None,
                            tweets: Vec::new(),
                            selected: 0,
                            scroll_offset: 0,
                            cursor_bottom: None,
                        });
                        self.spawn_user_profile(sn);
                    }
                }
            }
            KeyCode::Char('n') => {
                // New tweet
                self.push_screen(Screen::Compose {
                    mode: ComposeMode::NewTweet,
                    input: TextInput::new("What's happening?"),
                });
            }
            KeyCode::Char('d') => {
                // Open DMs
                self.push_screen(Screen::Loading {
                    message: "Loading messages...".to_string(),
                });
                self.spawn_dm_inbox();
            }
            KeyCode::Char('f') => self.toggle_like(),
            KeyCode::Char('t') => self.toggle_retweet(),
            KeyCode::Char('b') => self.toggle_bookmark(),
            KeyCode::Char(' ') => {
                if cursor_bottom.is_some() {
                    self.spawn_fetch_timeline(tab, true, cursor_bottom);
                }
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => self.pop_screen(),
            KeyCode::Char('j') | KeyCode::Down => {
                if let Screen::TweetDetail {
                    replies,
                    selected_reply,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    if !replies.is_empty() {
                        *selected_reply = (*selected_reply + 1).min(replies.len() - 1);
                        if *selected_reply > *scroll_offset + 8 {
                            *scroll_offset = selected_reply.saturating_sub(4);
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Screen::TweetDetail {
                    selected_reply,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    *selected_reply = selected_reply.saturating_sub(1);
                    if *selected_reply < *scroll_offset {
                        *scroll_offset = *selected_reply;
                    }
                }
            }
            KeyCode::Char('r') => {
                // Reply to main tweet
                if let Screen::TweetDetail {
                    main_tweet: Some(tweet),
                    ..
                } = self.current_screen()
                {
                    let tweet_id = tweet.id.clone();
                    let user = tweet.author.screen_name.clone();
                    self.push_screen(Screen::Compose {
                        mode: ComposeMode::Reply {
                            tweet_id,
                            reply_to_user: user,
                        },
                        input: TextInput::new("Tweet your reply"),
                    });
                }
            }
            KeyCode::Char('q') => {
                // Quote main tweet
                if let Screen::TweetDetail {
                    main_tweet: Some(tweet),
                    ..
                } = self.current_screen()
                {
                    let url = format!(
                        "https://x.com/{}/status/{}",
                        tweet.author.screen_name, tweet.id
                    );
                    self.push_screen(Screen::Compose {
                        mode: ComposeMode::Quote { tweet_url: url },
                        input: TextInput::new("Add a comment"),
                    });
                }
            }
            KeyCode::Char('f') => self.toggle_like_detail(),
            KeyCode::Char('p') => {
                // Open profile of main tweet author
                if let Screen::TweetDetail {
                    main_tweet: Some(tweet),
                    ..
                } = self.current_screen()
                {
                    let sn = tweet.author.screen_name.clone();
                    self.push_screen(Screen::Profile {
                        screen_name: sn.clone(),
                        user: None,
                        tweets: Vec::new(),
                        selected: 0,
                        scroll_offset: 0,
                        cursor_bottom: None,
                    });
                    self.spawn_user_profile(sn);
                }
            }
            _ => {}
        }
    }

    fn handle_profile_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => self.pop_screen(),
            KeyCode::Char('j') | KeyCode::Down => {
                if let Screen::Profile {
                    tweets,
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    if !tweets.is_empty() {
                        *selected = (*selected + 1).min(tweets.len() - 1);
                        if *selected > *scroll_offset + 10 {
                            *scroll_offset = selected.saturating_sub(5);
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Screen::Profile {
                    selected,
                    scroll_offset,
                    ..
                } = self.current_screen_mut()
                {
                    *selected = selected.saturating_sub(1);
                    if *selected < *scroll_offset {
                        *scroll_offset = *selected;
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Screen::Profile {
                    tweets, selected, ..
                } = self.current_screen()
                {
                    if let Some(tweet) = tweets.get(*selected) {
                        let tweet_id = tweet.id.clone();
                        self.push_screen(Screen::TweetDetail {
                            tweet_id: tweet_id.clone(),
                            main_tweet: None,
                            parents: Vec::new(),
                            replies: Vec::new(),
                            selected_reply: 0,
                            scroll_offset: 0,
                        });
                        self.spawn_tweet_detail(tweet_id);
                    }
                }
            }
            KeyCode::Char(' ') => {
                if let Screen::Profile {
                    user,
                    cursor_bottom,
                    ..
                } = self.current_screen()
                {
                    if let (Some(user), Some(cursor)) = (user, cursor_bottom) {
                        let user_id = user.id.clone();
                        let cursor = cursor.clone();
                        let tx = self.api_tx.clone();
                        let client = self.client.clone().unwrap();
                        tokio::spawn(async move {
                            match client.user_tweets(&user_id, 20, Some(&cursor)).await {
                                Ok(response) => {
                                    let _ = tx.send(ApiResult::UserTweets {
                                        response,
                                        user_id,
                                        append: true,
                                    });
                                }
                                Err(e) => {
                                    let _ = tx.send(ApiResult::Error(e.to_string()));
                                }
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_compose_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Screen::Compose { input, mode } = self.current_screen_mut() {
            let action = input.handle_key(key);
            match action {
                InputAction::Submit => {
                    if !input.is_empty() {
                        let text = input.text().to_string();
                        let mode = mode.clone();
                        // Can't borrow self mutably here, so clone what we need
                        drop(input);
                        self.spawn_send_tweet(text, mode);
                        self.pop_screen();
                        self.status_msg = Some("Sending tweet...".to_string());
                    }
                }
                InputAction::Cancel => {
                    self.pop_screen();
                }
                _ => {}
            }
        }
    }

    fn handle_dm_inbox_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => self.pop_screen(),
            KeyCode::Char('j') | KeyCode::Down => {
                if let Screen::DmInbox {
                    conversations,
                    selected,
                } = self.current_screen_mut()
                {
                    if !conversations.is_empty() {
                        *selected = (*selected + 1).min(conversations.len() - 1);
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Screen::DmInbox { selected, .. } = self.current_screen_mut() {
                    *selected = selected.saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                if let Screen::DmInbox {
                    conversations,
                    selected,
                } = self.current_screen()
                {
                    if let Some(convo) = conversations.get(*selected) {
                        let conv_id = convo.id.clone();
                        let name = format!(
                            "{} @{}",
                            convo.participant.name,
                            convo.participant.screen_name
                        );
                        let msgs = self
                            .dm_messages
                            .get(&conv_id)
                            .cloned()
                            .unwrap_or_default();
                        self.push_screen(Screen::DmConversation {
                            conversation_id: conv_id,
                            participant_name: name,
                            messages: msgs,
                            input: TextInput::new("Type a message..."),
                            scroll_offset: 0,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_dm_conversation_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Screen::DmConversation {
            conversation_id,
            input,
            ..
        } = self.current_screen_mut()
        {
            // Esc to go back (unless typing)
            if key.code == KeyCode::Esc {
                if input.content.is_empty() {
                    drop(input);
                    drop(conversation_id);
                    self.pop_screen();
                    return;
                } else {
                    input.clear();
                    return;
                }
            }

            let action = input.handle_key(key);
            match action {
                InputAction::Submit => {
                    if !input.is_empty() {
                        let text = input.text().to_string();
                        let conv_id = conversation_id.clone();
                        input.clear();
                        self.spawn_send_dm(conv_id, text);
                    }
                }
                InputAction::Cancel => {
                    drop(input);
                    drop(conversation_id);
                    self.pop_screen();
                }
                _ => {}
            }
        }
    }

    // ── Tweet action helpers ────────────────────────────────────────

    fn toggle_like(&mut self) {
        if let Screen::Timeline {
            tweets, selected, ..
        } = self.current_screen_mut()
        {
            if let Some(tweet) = tweets.get_mut(*selected) {
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
    }

    fn toggle_retweet(&mut self) {
        if let Screen::Timeline {
            tweets, selected, ..
        } = self.current_screen_mut()
        {
            if let Some(tweet) = tweets.get_mut(*selected) {
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
    }

    fn toggle_bookmark(&mut self) {
        if let Screen::Timeline {
            tweets, selected, ..
        } = self.current_screen_mut()
        {
            if let Some(tweet) = tweets.get_mut(*selected) {
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
    }

    fn toggle_like_detail(&mut self) {
        if let Screen::TweetDetail {
            main_tweet: Some(tweet),
            ..
        } = self.current_screen_mut()
        {
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
            self.spawn_tweet_action(id, action);
        }
    }
}

/// Parsed DM inbox data with conversations and their messages.
struct ParsedDmInbox {
    conversations: Vec<DmConversation>,
    /// All messages keyed by conversation_id.
    messages: std::collections::HashMap<String, Vec<DmMessage>>,
    /// The authenticated user's ID (inferred from participants).
    my_user_id: Option<String>,
}

/// Parse DM inbox from the v1.1 API response.
fn parse_dm_inbox(data: &serde_json::Value) -> ParsedDmInbox {
    let empty = ParsedDmInbox {
        conversations: Vec::new(),
        messages: std::collections::HashMap::new(),
        my_user_id: None,
    };

    let Some(inbox) = data.get("inbox_initial_state") else {
        return empty;
    };

    let users = inbox.get("users").and_then(|u| u.as_object());
    let user_ids: Vec<String> = users
        .map(|u| u.keys().cloned().collect())
        .unwrap_or_default();

    let convos_obj = inbox
        .get("conversations")
        .and_then(|c| c.as_object());

    // Collect all entries (messages) grouped by conversation
    let mut messages_by_convo: std::collections::HashMap<String, Vec<DmMessage>> =
        std::collections::HashMap::new();

    if let Some(entries) = inbox.get("entries").and_then(|e| e.as_array()) {
        for entry in entries {
            if let Some(msg) = entry.get("message") {
                let conv_id = msg
                    .get("conversation_id")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default()
                    .to_string();
                let msg_data = msg.get("message_data");
                let text = msg_data
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string();
                let sender_id = msg_data
                    .and_then(|d| d.get("sender_id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                let time_ms = msg
                    .get("time")
                    .and_then(|t| t.as_str())
                    .and_then(|s| s.parse::<i64>().ok());
                let created_at = time_ms.and_then(|ms| {
                    chrono::DateTime::from_timestamp_millis(ms)
                });

                messages_by_convo
                    .entry(conv_id)
                    .or_default()
                    .push(DmMessage {
                        id: msg
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        text,
                        sender_id,
                        created_at,
                    });
            }
        }
    }

    // Sort messages by ID (chronological)
    for msgs in messages_by_convo.values_mut() {
        msgs.sort_by(|a, b| a.id.cmp(&b.id));
    }

    // Infer my user_id: the participant that appears in all conversations
    let mut my_user_id: Option<String> = None;
    let Some(convos_map) = convos_obj else {
        return ParsedDmInbox {
            conversations: Vec::new(),
            messages: messages_by_convo,
            my_user_id,
        };
    };

    // In a 1-on-1 convo, both participants are listed. The one that
    // shows up in every conversation is "me".
    let all_participant_ids: Vec<Vec<String>> = convos_map
        .values()
        .map(|c| {
            c.get("participants")
                .and_then(|p| p.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| p.get("user_id").and_then(|u| u.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    if let Some(first) = all_participant_ids.first() {
        for uid in first {
            if all_participant_ids
                .iter()
                .all(|pids| pids.contains(uid))
            {
                my_user_id = Some(uid.clone());
                break;
            }
        }
    }
    // Fallback: if only 1 convo, pick the first participant as "me" guess
    // (we'll refine later when we have the authenticated user info)

    let mut result = Vec::new();

    for convo in convos_map.values() {
        let convo_id = convo
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let participants: Vec<String> = convo
            .get("participants")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("user_id").and_then(|u| u.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Find the other user (not me)
        let other_user_id = participants
            .iter()
            .find(|uid| my_user_id.as_ref() != Some(uid))
            .or_else(|| participants.first()) // fallback
            .cloned()
            .unwrap_or_default();

        let participant = make_user_from_dm_data(&other_user_id, users);

        let last_message = messages_by_convo
            .get(&convo_id)
            .and_then(|msgs| msgs.last())
            .cloned();

        // Read status
        let my_last_read = convo
            .get("participants")
            .and_then(|p| p.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|p| {
                        p.get("user_id").and_then(|u| u.as_str())
                            == my_user_id.as_deref()
                    })
                    .and_then(|p| p.get("last_read_event_id").and_then(|l| l.as_str()))
            });
        let max_entry = convo
            .get("max_entry_id")
            .and_then(|m| m.as_str());
        let unread = match (my_last_read, max_entry) {
            (Some(read), Some(max)) => read != max,
            _ => false,
        };

        result.push(DmConversation {
            id: convo_id,
            participant,
            last_message,
            unread,
        });
    }

    // Sort by most recent message
    result.sort_by(|a, b| {
        let a_id = a.last_message.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        let b_id = b.last_message.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        b_id.cmp(a_id)
    });

    ParsedDmInbox {
        conversations: result,
        messages: messages_by_convo,
        my_user_id,
    }
}

fn make_user_from_dm_data(
    user_id: &str,
    users: Option<&serde_json::Map<String, serde_json::Value>>,
) -> User {
    if let Some(user_data) = users.and_then(|u| u.get(user_id)) {
        User {
            id: user_id.to_string(),
            name: user_data
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
            screen_name: user_data
                .get("screen_name")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            description: None,
            followers_count: 0,
            following_count: 0,
            tweet_count: 0,
            verified: false,
            profile_image_url: user_data
                .get("profile_image_url_https")
                .and_then(|u| u.as_str())
                .map(String::from),
            profile_banner_url: None,
            created_at: None,
            following: false,
            followed_by: false,
        }
    } else {
        User {
            id: user_id.to_string(),
            name: "Unknown".to_string(),
            screen_name: "unknown".to_string(),
            description: None,
            followers_count: 0,
            following_count: 0,
            tweet_count: 0,
            verified: false,
            profile_image_url: None,
            profile_banner_url: None,
            created_at: None,
            following: false,
            followed_by: false,
        }
    }
}
