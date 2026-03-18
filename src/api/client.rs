use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::graphql::{self, GraphQLOpsCache, fallback_query_ids};
use super::models::*;
use super::rate_limit;
use crate::auth::Credentials;
use crate::config::Config;

const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const GRAPHQL_BASE: &str = "https://x.com/i/api/graphql";
const API_V1_BASE: &str = "https://x.com/i/api/1.1";
const API_V2_BASE: &str = "https://x.com/i/api/2";

/// The main API client for X/Twitter.
pub struct XClient {
    http: reqwest::Client,
    credentials: Credentials,
    config: Config,
    ops_cache: Arc<RwLock<GraphQLOpsCache>>,
}

impl XClient {
    pub async fn new(credentials: Credentials, config: Config) -> Result<Self> {
        let http = build_http_client(&config)?;

        // Load or fetch GraphQL operations
        let ops_cache = match GraphQLOpsCache::load_cached() {
            Some(cache) => {
                tracing::debug!("Loaded {} cached GraphQL operations", cache.operations.len());
                cache
            }
            None => graphql::extract_operations(&http).await?,
        };

        Ok(Self {
            http,
            credentials,
            config,
            ops_cache: Arc::new(RwLock::new(ops_cache)),
        })
    }

    /// Build default headers for all API requests.
    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {BEARER_TOKEN}")).unwrap(),
        );
        headers.insert(
            "x-csrf-token",
            HeaderValue::from_str(&self.credentials.ct0).unwrap(),
        );
        headers.insert(
            "x-twitter-auth-type",
            HeaderValue::from_static("OAuth2Session"),
        );
        headers.insert(
            "x-twitter-active-user",
            HeaderValue::from_static("yes"),
        );
        headers.insert(
            "x-twitter-client-language",
            HeaderValue::from_static("en"),
        );
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/json"),
        );
        headers.insert("referer", HeaderValue::from_static("https://x.com/"));
        headers.insert("origin", HeaderValue::from_static("https://x.com"));

        // Cookie header
        let cookie = format!(
            "auth_token={}; ct0={}",
            self.credentials.auth_token, self.credentials.ct0
        );
        headers.insert(
            "cookie",
            HeaderValue::from_str(&cookie).unwrap(),
        );

        headers
    }

    /// Get the query ID for an operation, with fallback support.
    async fn get_query_id(&self, operation: &str) -> Result<String> {
        let cache = self.ops_cache.read().await;
        if let Some(op) = cache.get(operation) {
            return Ok(op.query_id.clone());
        }

        // Check fallbacks
        if let Some(id) = fallback_query_ids().get(operation) {
            return Ok(id.to_string());
        }

        anyhow::bail!("Unknown GraphQL operation: {operation}")
    }

    /// Refresh GraphQL operations (on 404/422).
    async fn refresh_operations(&self) -> Result<()> {
        tracing::info!("Refreshing GraphQL operations...");
        GraphQLOpsCache::invalidate();
        let new_cache = graphql::extract_operations(&self.http).await?;
        let mut cache = self.ops_cache.write().await;
        *cache = new_cache;
        Ok(())
    }

    /// Make a GraphQL GET request (for reads).
    async fn graphql_get(
        &self,
        operation: &str,
        variables: Value,
        features: Option<Value>,
    ) -> Result<Value> {
        let query_id = self.get_query_id(operation).await?;
        let url = format!("{GRAPHQL_BASE}/{query_id}/{operation}");

        let mut params = vec![("variables", serde_json::to_string(&variables)?)];
        if let Some(features) = features {
            params.push(("features", serde_json::to_string(&features)?));
        }

        let headers = self.default_headers();

        for attempt in 0..=self.config.max_retries {
            let resp = self
                .http
                .get(&url)
                .headers(headers.clone())
                .query(&params)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                return Ok(resp.json().await?);
            }

            // Stale endpoint recovery
            if (status.as_u16() == 404 || status.as_u16() == 422) && attempt == 0 {
                tracing::warn!("{operation} returned {status}, refreshing operations...");
                self.refresh_operations().await?;
                // Retry with new query ID
                let new_query_id = self.get_query_id(operation).await?;
                let new_url = format!("{GRAPHQL_BASE}/{new_query_id}/{operation}");
                let resp = self
                    .http
                    .get(&new_url)
                    .headers(headers.clone())
                    .query(&params)
                    .send()
                    .await?;
                if resp.status().is_success() {
                    return Ok(resp.json().await?);
                }
            }

            if attempt < self.config.max_retries {
                let delay = rate_limit::backoff(attempt);
                tracing::warn!(
                    "{operation} failed with {status}, retrying in {:?}...",
                    delay
                );
                tokio::time::sleep(delay).await;
            }
        }

        anyhow::bail!("GraphQL GET {operation} failed after {} retries", self.config.max_retries)
    }

    /// Make a GraphQL POST request (for writes).
    async fn graphql_post(
        &self,
        operation: &str,
        variables: Value,
        features: Option<Value>,
    ) -> Result<Value> {
        let query_id = self.get_query_id(operation).await?;
        let url = format!("{GRAPHQL_BASE}/{query_id}/{operation}");

        let mut body = serde_json::json!({
            "variables": variables,
            "queryId": query_id,
        });
        if let Some(features) = features {
            body["features"] = features;
        }

        let headers = self.default_headers();

        for attempt in 0..=self.config.max_retries {
            // Rate limit for writes
            let delay = rate_limit::write_delay(
                self.config.write_delay_min_ms,
                self.config.write_delay_max_ms,
            );
            tokio::time::sleep(delay).await;

            let resp = self
                .http
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                return Ok(resp.json().await?);
            }

            if (status.as_u16() == 404 || status.as_u16() == 422) && attempt == 0 {
                tracing::warn!("{operation} returned {status}, refreshing operations...");
                self.refresh_operations().await?;
                let new_query_id = self.get_query_id(operation).await?;
                let new_url = format!("{GRAPHQL_BASE}/{new_query_id}/{operation}");
                body["queryId"] = Value::String(new_query_id);
                let resp = self
                    .http
                    .post(&new_url)
                    .headers(headers.clone())
                    .json(&body)
                    .send()
                    .await?;
                if resp.status().is_success() {
                    return Ok(resp.json().await?);
                }
            }

            if attempt < self.config.max_retries {
                let delay = rate_limit::backoff(attempt);
                tracing::warn!("{operation} failed with {status}, retrying in {:?}...", delay);
                tokio::time::sleep(delay).await;
            }
        }

        anyhow::bail!("GraphQL POST {operation} failed after {} retries", self.config.max_retries)
    }

    /// REST v1.1 POST request.
    async fn rest_post(&self, path: &str, form: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{API_V1_BASE}/{path}");
        let mut headers = self.default_headers();
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );

        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .form(form)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("REST POST {path} failed: {status} - {body}")
        }
    }

    /// REST v1.1 GET request.
    async fn rest_get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{API_V1_BASE}/{path}");
        let headers = self.default_headers();

        let resp = self
            .http
            .get(&url)
            .headers(headers)
            .query(params)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("REST GET {path} failed: {status} - {body}")
        }
    }

    // ── Timeline endpoints ──────────────────────────────────────────

    /// Fetch the "For You" timeline.
    pub async fn home_timeline(&self, count: u32, cursor: Option<&str>) -> Result<TimelineResponse> {
        let mut vars = serde_json::json!({
            "count": count,
            "includePromotedContent": false,
            "latestControlAvailable": true,
            "requestContext": "launch",
        });
        if let Some(cursor) = cursor {
            vars["cursor"] = Value::String(cursor.to_string());
        }

        let data = self.graphql_get("HomeTimeline", vars, Some(default_features())).await?;
        parse_timeline(&data, &["data", "home", "home_timeline_urt", "instructions"])
    }

    /// Fetch the "Following" (chronological) timeline.
    pub async fn home_latest(&self, count: u32, cursor: Option<&str>) -> Result<TimelineResponse> {
        let mut vars = serde_json::json!({
            "count": count,
            "includePromotedContent": false,
            "latestControlAvailable": true,
            "requestContext": "launch",
        });
        if let Some(cursor) = cursor {
            vars["cursor"] = Value::String(cursor.to_string());
        }

        let data = self.graphql_get("HomeLatestTimeline", vars, Some(default_features())).await?;
        parse_timeline(&data, &["data", "home", "home_timeline_urt", "instructions"])
    }

    /// Search tweets.
    pub async fn search(
        &self,
        query: &str,
        count: u32,
        cursor: Option<&str>,
    ) -> Result<TimelineResponse> {
        let mut vars = serde_json::json!({
            "rawQuery": query,
            "count": count,
            "querySource": "typed_query",
            "product": "Top",
        });
        if let Some(cursor) = cursor {
            vars["cursor"] = Value::String(cursor.to_string());
        }

        let data = self
            .graphql_get("SearchTimeline", vars, Some(default_features()))
            .await?;
        parse_timeline(
            &data,
            &[
                "data",
                "search_by_raw_query",
                "search_timeline",
                "timeline",
                "instructions",
            ],
        )
    }

    /// Get a user's profile by screen name.
    pub async fn user_by_screen_name(&self, screen_name: &str) -> Result<User> {
        let vars = serde_json::json!({
            "screen_name": screen_name,
            "withSafetyModeUserFields": true,
        });

        let data = self
            .graphql_get("UserByScreenName", vars, Some(default_features()))
            .await?;

        let result = data
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("result"))
            .context("User not found in response")?;

        User::from_api_result(result).context("Failed to parse user")
    }

    /// Get a user's tweets.
    pub async fn user_tweets(
        &self,
        user_id: &str,
        count: u32,
        cursor: Option<&str>,
    ) -> Result<TimelineResponse> {
        let mut vars = serde_json::json!({
            "userId": user_id,
            "count": count,
            "includePromotedContent": false,
            "withQuickPromoteEligibilityTweetFields": true,
            "withVoice": true,
            "withV2Timeline": true,
        });
        if let Some(cursor) = cursor {
            vars["cursor"] = Value::String(cursor.to_string());
        }

        let data = self
            .graphql_get("UserTweets", vars, Some(default_features()))
            .await?;
        parse_timeline(
            &data,
            &[
                "data",
                "user",
                "result",
                "timeline_v2",
                "timeline",
                "instructions",
            ],
        )
    }

    /// Get tweet detail with conversation thread.
    pub async fn tweet_detail(&self, tweet_id: &str) -> Result<ThreadResponse> {
        let vars = serde_json::json!({
            "focalTweetId": tweet_id,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": false,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true,
        });

        let data = self
            .graphql_get("TweetDetail", vars, Some(default_features()))
            .await?;

        parse_thread(&data, tweet_id)
    }

    /// Get bookmarks.
    pub async fn bookmarks(&self, count: u32, cursor: Option<&str>) -> Result<TimelineResponse> {
        let mut vars = serde_json::json!({
            "count": count,
            "includePromotedContent": false,
        });
        if let Some(cursor) = cursor {
            vars["cursor"] = Value::String(cursor.to_string());
        }

        let data = self
            .graphql_get("BookmarkSearchTimeline", vars, Some(default_features()))
            .await?;
        parse_timeline(
            &data,
            &[
                "data",
                "bookmark_timeline_v2",
                "timeline",
                "instructions",
            ],
        )
    }

    // ── Write endpoints ─────────────────────────────────────────────

    /// Post a new tweet.
    pub async fn create_tweet(
        &self,
        text: &str,
        reply_to: Option<&str>,
        quote_url: Option<&str>,
        media_ids: Vec<String>,
    ) -> Result<Value> {
        let mut vars = serde_json::json!({
            "tweet_text": text,
            "dark_request": false,
            "semantic_annotation_ids": [],
        });

        if let Some(reply_to) = reply_to {
            vars["reply"] = serde_json::json!({
                "in_reply_to_tweet_id": reply_to,
                "exclude_reply_user_ids": [],
            });
        }

        if let Some(url) = quote_url {
            vars["attachment_url"] = Value::String(url.to_string());
        }

        if !media_ids.is_empty() {
            let entities: Vec<Value> = media_ids
                .into_iter()
                .map(|id| serde_json::json!({"media_id": id, "tagged_users": []}))
                .collect();
            vars["media"] = serde_json::json!({
                "media_entities": entities,
                "possibly_sensitive": false,
            });
        }

        self.graphql_post("CreateTweet", vars, Some(default_features()))
            .await
    }

    /// Delete a tweet.
    pub async fn delete_tweet(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("DeleteTweet", vars, None).await
    }

    /// Like a tweet.
    pub async fn like(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("FavoriteTweet", vars, None).await
    }

    /// Unlike a tweet.
    pub async fn unlike(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("UnfavoriteTweet", vars, None).await
    }

    /// Retweet.
    pub async fn retweet(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("CreateRetweet", vars, None).await
    }

    /// Unretweet.
    pub async fn unretweet(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"source_tweet_id": tweet_id});
        self.graphql_post("DeleteRetweet", vars, None).await
    }

    /// Bookmark a tweet.
    pub async fn bookmark(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("CreateBookmark", vars, None).await
    }

    /// Unbookmark a tweet.
    pub async fn unbookmark(&self, tweet_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"tweet_id": tweet_id});
        self.graphql_post("DeleteBookmark", vars, None).await
    }

    /// Follow a user.
    pub async fn follow(&self, user_id: &str) -> Result<Value> {
        self.rest_post(
            "friendships/create.json",
            &[("user_id", user_id), ("include_profile_interstitial_type", "1")],
        )
        .await
    }

    /// Unfollow a user.
    pub async fn unfollow(&self, user_id: &str) -> Result<Value> {
        self.rest_post(
            "friendships/destroy.json",
            &[("user_id", user_id)],
        )
        .await
    }

    /// Block a user.
    pub async fn block(&self, user_id: &str) -> Result<Value> {
        self.rest_post("blocks/create.json", &[("user_id", user_id)])
            .await
    }

    /// Unblock a user.
    pub async fn unblock(&self, user_id: &str) -> Result<Value> {
        self.rest_post("blocks/destroy.json", &[("user_id", user_id)])
            .await
    }

    // ── DM endpoints ───────────────────────────────────────────────

    /// Fetch DM inbox via REST v1.1 with optional cursor for pagination.
    pub async fn dm_inbox(&self, cursor: Option<&str>) -> Result<Value> {
        let mut params = vec![
            ("nsfw_filtering_enabled", "false"),
            ("filter_low_quality", "false"),
            ("include_quality", "all"),
            ("dm_secret_conversations_enabled", "false"),
            ("krs_registration_enabled", "true"),
            ("cards_platform", "Web-12"),
            ("include_cards", "1"),
            ("include_ext_alt_text", "true"),
            ("include_quote_count", "true"),
            ("include_reply_count", "1"),
            ("tweet_mode", "extended"),
            ("include_ext_collab_control", "true"),
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl"),
        ];

        // For pagination, use the same endpoint with cursor and max_id
        let path = if let Some(c) = cursor {
            params.push(("cursor", c));
            params.push(("max_id", c));
            "dm/inbox_initial_state.json"
        } else {
            "dm/inbox_initial_state.json"
        };

        self.rest_get(path, &params).await
    }

    /// Send a DM to a conversation.
    pub async fn send_dm(&self, conversation_id: &str, text: &str) -> Result<Value> {
        let request_id = format!("{:x}{:x}", rand::random::<u64>(), rand::random::<u64>());
        let vars = serde_json::json!({
            "target": {"participant_ids": []},
            "requestId": request_id,
            "dmComposerRequest": {
                "conversationId": conversation_id,
                "text": {"text": text},
            },
        });
        self.graphql_post("useSendMessageMutation", vars, None).await
    }

    /// Delete a DM by message ID.
    pub async fn delete_dm(&self, message_id: &str) -> Result<Value> {
        let vars = serde_json::json!({"messageId": message_id});
        self.graphql_post("DMMessageDeleteMutation", vars, None).await
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn build_http_client(config: &Config) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36");

    if let Some(ref proxy) = config.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }

    builder.build().context("Failed to build HTTP client")
}

/// Default feature flags sent with most GraphQL requests.
fn default_features() -> Value {
    serde_json::json!({
        "rweb_tipjar_consumption_enabled": true,
        "responsive_web_graphql_exclude_directive_enabled": true,
        "verified_phone_label_enabled": false,
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "communities_web_enable_tweet_community_results_fetch": true,
        "c9s_tweet_anatomy_moderator_badge_enabled": true,
        "articles_preview_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": true,
        "tweet_awards_web_tipping_enabled": false,
        "creator_subscriptions_quote_tweet_preview_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "rweb_video_timestamps_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_enhance_cards_enabled": false,
    })
}

/// Parse a timeline response from the nested instruction format.
fn parse_timeline(data: &Value, path: &[&str]) -> Result<TimelineResponse> {
    let mut current = data;
    for key in path {
        current = current
            .get(key)
            .with_context(|| format!("Missing key '{key}' in timeline response"))?;
    }

    let instructions = current
        .as_array()
        .context("Instructions is not an array")?;

    let mut tweets = Vec::new();
    let mut cursor_top = None;
    let mut cursor_bottom = None;

    for instruction in instructions {
        let instruction_type = instruction
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();

        match instruction_type {
            "TimelineAddEntries" | "TimelineAddToModule" => {
                if let Some(entries) = instruction
                    .get("entries")
                    .and_then(|e| e.as_array())
                {
                    for entry in entries {
                        let entry_id = entry
                            .get("entryId")
                            .and_then(|e| e.as_str())
                            .unwrap_or_default();

                        if entry_id.starts_with("cursor-top") {
                            cursor_top = entry
                                .get("content")
                                .and_then(|c| c.get("value"))
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        } else if entry_id.starts_with("cursor-bottom") {
                            cursor_bottom = entry
                                .get("content")
                                .and_then(|c| c.get("value"))
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        } else if let Some(tweet_result) = entry
                            .get("content")
                            .and_then(|c| c.get("itemContent"))
                            .and_then(|i| i.get("tweet_results"))
                            .and_then(|t| t.get("result"))
                        {
                            if let Some(tweet) = Tweet::from_api_result(tweet_result) {
                                tweets.push(tweet);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(TimelineResponse {
        tweets,
        cursor_top,
        cursor_bottom,
    })
}

/// Parse a thread response from TweetDetail.
fn parse_thread(data: &Value, focal_tweet_id: &str) -> Result<ThreadResponse> {
    let instructions = data
        .get("data")
        .and_then(|d| d.get("threaded_conversation_with_injections_v2"))
        .and_then(|t| t.get("instructions"))
        .and_then(|i| i.as_array())
        .context("Missing thread instructions")?;

    let mut main_tweet = None;
    let mut parents = Vec::new();
    let mut replies = Vec::new();
    let mut found_focal = false;

    for instruction in instructions {
        if instruction.get("type").and_then(|t| t.as_str()) != Some("TimelineAddEntries") {
            continue;
        }
        let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) else {
            continue;
        };

        for entry in entries {
            // Single tweet entry
            if let Some(tweet_result) = entry
                .get("content")
                .and_then(|c| c.get("itemContent"))
                .and_then(|i| i.get("tweet_results"))
                .and_then(|t| t.get("result"))
            {
                if let Some(tweet) = Tweet::from_api_result(tweet_result) {
                    if tweet.id == focal_tweet_id {
                        main_tweet = Some(tweet);
                        found_focal = true;
                    } else if found_focal {
                        replies.push(tweet);
                    } else {
                        parents.push(tweet);
                    }
                }
            }

            // Conversation module (thread of replies)
            if let Some(items) = entry
                .get("content")
                .and_then(|c| c.get("items"))
                .and_then(|i| i.as_array())
            {
                for item in items {
                    if let Some(tweet_result) = item
                        .get("item")
                        .and_then(|i| i.get("itemContent"))
                        .and_then(|i| i.get("tweet_results"))
                        .and_then(|t| t.get("result"))
                    {
                        if let Some(tweet) = Tweet::from_api_result(tweet_result) {
                            if tweet.id == focal_tweet_id {
                                main_tweet = Some(tweet);
                                found_focal = true;
                            } else if found_focal {
                                replies.push(tweet);
                            } else {
                                parents.push(tweet);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ThreadResponse {
        main_tweet: main_tweet.context("Focal tweet not found in thread")?,
        parents,
        replies,
    })
}
