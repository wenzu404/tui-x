use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Parsed tweet from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    pub id: String,
    pub text: String,
    pub author: User,
    pub created_at: Option<DateTime<Utc>>,
    pub reply_count: u64,
    pub retweet_count: u64,
    pub like_count: u64,
    pub view_count: Option<u64>,
    pub bookmark_count: u64,
    pub quote_count: u64,
    pub is_retweet: bool,
    pub retweeted_by: Option<String>,
    pub in_reply_to_id: Option<String>,
    pub quoted_tweet: Option<Box<Tweet>>,
    pub media: Vec<TweetMedia>,
    pub favorited: bool,
    pub retweeted: bool,
    pub bookmarked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TweetMedia {
    pub media_type: MediaType,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub alt_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaType {
    Photo,
    Video,
    AnimatedGif,
}

/// Parsed user from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub screen_name: String,
    pub description: Option<String>,
    pub followers_count: u64,
    pub following_count: u64,
    pub tweet_count: u64,
    pub verified: bool,
    pub profile_image_url: Option<String>,
    pub profile_banner_url: Option<String>,
    pub created_at: Option<String>,
    pub following: bool,
    pub followed_by: bool,
}

/// A timeline response with tweets and cursor for pagination.
#[derive(Debug, Clone)]
pub struct TimelineResponse {
    pub tweets: Vec<Tweet>,
    pub cursor_top: Option<String>,
    pub cursor_bottom: Option<String>,
}

/// Conversation thread.
#[derive(Debug, Clone)]
pub struct ThreadResponse {
    pub main_tweet: Tweet,
    pub parents: Vec<Tweet>,
    pub replies: Vec<Tweet>,
}

/// DM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmConversation {
    pub id: String,
    pub participant: User,
    pub last_message: Option<DmMessage>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: String,
    pub text: String,
    pub sender_id: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl Tweet {
    /// Parse a tweet from the nested API JSON structure.
    pub fn from_api_result(value: &serde_json::Value) -> Option<Self> {
        // Handle tombstones and unavailable tweets
        let result = if value.get("tweet").is_some() {
            value.get("tweet")?
        } else {
            value
        };

        // Handle TweetWithVisibilityResults wrapper
        let result = if result.get("__typename").and_then(|t| t.as_str())
            == Some("TweetWithVisibilityResults")
        {
            result.get("tweet")?
        } else {
            result
        };

        let legacy = result.get("legacy")?;
        let core = result.get("core")?;
        let user_result = core
            .get("user_results")
            .and_then(|u| u.get("result"))?;

        let author = User::from_api_result(user_result)?;
        let id = legacy
            .get("id_str")
            .and_then(|v| v.as_str())?
            .to_string();

        let text = legacy
            .get("full_text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let created_at = legacy
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_str(s, "%a %b %d %H:%M:%S %z %Y").ok())
            .map(|dt| dt.with_timezone(&Utc));

        let view_count = result
            .get("views")
            .and_then(|v| v.get("count"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        let media = Self::parse_media(legacy);

        let quoted_tweet = result
            .get("quoted_status_result")
            .and_then(|q| q.get("result"))
            .and_then(Tweet::from_api_result)
            .map(Box::new);

        let retweeted_status = legacy.get("retweeted_status_result")
            .and_then(|r| r.get("result"));

        let (is_retweet, retweeted_by) = if retweeted_status.is_some() {
            (true, Some(author.screen_name.clone()))
        } else {
            (false, None)
        };

        Some(Tweet {
            id,
            text,
            author,
            created_at,
            reply_count: get_u64(legacy, "reply_count"),
            retweet_count: get_u64(legacy, "retweet_count"),
            like_count: get_u64(legacy, "favorite_count"),
            view_count,
            bookmark_count: get_u64(legacy, "bookmark_count"),
            quote_count: get_u64(legacy, "quote_count"),
            is_retweet,
            retweeted_by,
            in_reply_to_id: legacy
                .get("in_reply_to_status_id_str")
                .and_then(|v| v.as_str())
                .map(String::from),
            quoted_tweet,
            media,
            favorited: legacy.get("favorited").and_then(|v| v.as_bool()).unwrap_or(false),
            retweeted: legacy.get("retweeted").and_then(|v| v.as_bool()).unwrap_or(false),
            bookmarked: legacy.get("bookmarked").and_then(|v| v.as_bool()).unwrap_or(false),
        })
    }

    fn parse_media(legacy: &serde_json::Value) -> Vec<TweetMedia> {
        let Some(entities) = legacy
            .get("extended_entities")
            .or_else(|| legacy.get("entities"))
            .and_then(|e| e.get("media"))
            .and_then(|m| m.as_array())
        else {
            return Vec::new();
        };

        entities
            .iter()
            .filter_map(|m| {
                let media_type = match m.get("type").and_then(|t| t.as_str())? {
                    "photo" => MediaType::Photo,
                    "video" => MediaType::Video,
                    "animated_gif" => MediaType::AnimatedGif,
                    _ => return None,
                };

                let url = match media_type {
                    MediaType::Photo => {
                        m.get("media_url_https").and_then(|u| u.as_str())?.to_string()
                    }
                    MediaType::Video | MediaType::AnimatedGif => {
                        // Get highest bitrate variant
                        m.get("video_info")
                            .and_then(|v| v.get("variants"))
                            .and_then(|v| v.as_array())
                            .and_then(|variants| {
                                variants
                                    .iter()
                                    .filter(|v| {
                                        v.get("content_type").and_then(|c| c.as_str())
                                            == Some("video/mp4")
                                    })
                                    .max_by_key(|v| {
                                        v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0)
                                    })
                                    .and_then(|v| v.get("url").and_then(|u| u.as_str()))
                            })?
                            .to_string()
                    }
                };

                Some(TweetMedia {
                    media_type,
                    thumbnail_url: m
                        .get("media_url_https")
                        .and_then(|u| u.as_str())
                        .map(String::from),
                    width: m
                        .get("original_info")
                        .and_then(|o| o.get("width"))
                        .and_then(|w| w.as_u64())
                        .map(|w| w as u32),
                    height: m
                        .get("original_info")
                        .and_then(|o| o.get("height"))
                        .and_then(|h| h.as_u64())
                        .map(|h| h as u32),
                    alt_text: m
                        .get("ext_alt_text")
                        .and_then(|a| a.as_str())
                        .map(String::from),
                    url,
                })
            })
            .collect()
    }
}

impl User {
    pub fn from_api_result(value: &serde_json::Value) -> Option<Self> {
        let legacy = value.get("legacy");
        let core = value.get("core");
        let relationships = value.get("relationship_perspectives");

        // name/screen_name: new API puts them in "core", old API in "legacy"
        let name = core
            .and_then(|c| c.get("name"))
            .or_else(|| legacy.and_then(|l| l.get("name")))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let screen_name = core
            .and_then(|c| c.get("screen_name"))
            .or_else(|| legacy.and_then(|l| l.get("screen_name")))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        // created_at: new API in "core", old in "legacy"
        let created_at = core
            .and_then(|c| c.get("created_at"))
            .or_else(|| legacy.and_then(|l| l.get("created_at")))
            .and_then(|v| v.as_str())
            .map(String::from);

        // description: new API in "profile_bio.description", old in "legacy.description"
        let description = value
            .get("profile_bio")
            .and_then(|pb| pb.get("description"))
            .or_else(|| legacy.and_then(|l| l.get("description")))
            .and_then(|v| v.as_str())
            .map(String::from);

        // avatar: new API in "avatar.image_url", old in "legacy.profile_image_url_https"
        let profile_image_url = value
            .get("avatar")
            .and_then(|a| a.get("image_url"))
            .or_else(|| legacy.and_then(|l| l.get("profile_image_url_https")))
            .and_then(|v| v.as_str())
            .map(String::from);

        let profile_banner_url = legacy
            .and_then(|l| l.get("profile_banner_url"))
            .and_then(|v| v.as_str())
            .map(String::from);

        // Counts are still in legacy
        let legacy_ref = legacy.unwrap_or(&serde_json::Value::Null);

        // following/followed_by: new API in "relationship_perspectives", old in "legacy"
        let following = relationships
            .and_then(|r| r.get("following"))
            .or_else(|| legacy.and_then(|l| l.get("following")))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let followed_by = relationships
            .and_then(|r| r.get("followed_by"))
            .or_else(|| legacy.and_then(|l| l.get("followed_by")))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Some(User {
            id: value
                .get("rest_id")
                .and_then(|v| v.as_str())?
                .to_string(),
            name,
            screen_name,
            description,
            followers_count: get_u64(legacy_ref, "followers_count"),
            following_count: get_u64(legacy_ref, "friends_count"),
            tweet_count: get_u64(legacy_ref, "statuses_count"),
            verified: value
                .get("is_blue_verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            profile_image_url,
            profile_banner_url,
            created_at,
            following,
            followed_by,
        })
    }
}

fn get_u64(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}
