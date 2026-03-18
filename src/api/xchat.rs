//! XChat (new DM system) — Thrift decoding and GraphQL queries.

use anyhow::{Context, Result};
use serde_json::Value;

use super::models::*;

// ── Thrift Binary Protocol decoder (minimal, for XChat messages) ────

/// Thrift field types we care about.
const THRIFT_STOP: u8 = 0;
const THRIFT_BOOL: u8 = 2;
const THRIFT_I16: u8 = 6;
const THRIFT_I32: u8 = 8;
const THRIFT_I64: u8 = 10;
const THRIFT_STRING: u8 = 11;
const THRIFT_STRUCT: u8 = 12;
const THRIFT_LIST: u8 = 15;

struct ThriftReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ThriftReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_byte(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    fn read_i16(&mut self) -> Option<i16> {
        if self.pos + 2 <= self.data.len() {
            let val = i16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
            self.pos += 2;
            Some(val)
        } else {
            None
        }
    }

    fn read_i32(&mut self) -> Option<i32> {
        if self.pos + 4 <= self.data.len() {
            let val = i32::from_be_bytes([
                self.data[self.pos],
                self.data[self.pos + 1],
                self.data[self.pos + 2],
                self.data[self.pos + 3],
            ]);
            self.pos += 4;
            Some(val)
        } else {
            None
        }
    }

    fn read_i64(&mut self) -> Option<i64> {
        if self.pos + 8 <= self.data.len() {
            let val = i64::from_be_bytes([
                self.data[self.pos],
                self.data[self.pos + 1],
                self.data[self.pos + 2],
                self.data[self.pos + 3],
                self.data[self.pos + 4],
                self.data[self.pos + 5],
                self.data[self.pos + 6],
                self.data[self.pos + 7],
            ]);
            self.pos += 8;
            Some(val)
        } else {
            None
        }
    }

    fn read_string(&mut self) -> Option<String> {
        let len = self.read_i32()? as usize;
        if len > 100_000 || self.pos + len > self.data.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).to_string();
        self.pos += len;
        Some(s)
    }

    fn read_bytes(&mut self) -> Option<Vec<u8>> {
        let len = self.read_i32()? as usize;
        if len > 100_000 || self.pos + len > self.data.len() {
            return None;
        }
        let b = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Some(b)
    }

    /// Skip a thrift value of the given type.
    fn skip(&mut self, field_type: u8) -> Option<()> {
        match field_type {
            THRIFT_BOOL => { self.read_byte()?; }
            THRIFT_I16 => { self.read_i16()?; }
            THRIFT_I32 => { self.read_i32()?; }
            THRIFT_I64 => { self.read_i64()?; }
            THRIFT_STRING => { self.read_string()?; }
            THRIFT_STRUCT => { self.skip_struct()?; }
            THRIFT_LIST => {
                let elem_type = self.read_byte()?;
                let count = self.read_i32()?;
                for _ in 0..count {
                    self.skip(elem_type)?;
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn skip_struct(&mut self) -> Option<()> {
        loop {
            let field_type = self.read_byte()?;
            if field_type == THRIFT_STOP {
                return Some(());
            }
            let _field_id = self.read_i16()?;
            self.skip(field_type)?;
        }
    }
}

/// A decoded XChat message event.
#[derive(Debug)]
pub struct XChatMessageEvent {
    pub message_id: String,
    pub sender_id: String,
    pub conversation_id: String,
    pub timestamp_ms: String,
    pub text: Option<String>,
    pub shared_tweet_id: Option<String>,
    pub shared_tweet_url: Option<String>,
    pub is_encrypted: bool,
}

/// Decode a Thrift-serialized XChat message event from base64.
pub fn decode_message_event(b64: &str) -> Option<XChatMessageEvent> {
    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok()?;
    let mut reader = ThriftReader::new(&data);

    let mut message_id = String::new();
    let mut uuid = String::new();
    let mut sender_id = String::new();
    let mut conversation_id = String::new();
    let mut jwt_token: Option<String> = None;
    let mut timestamp_ms = String::new();
    let mut text: Option<String> = None;
    let mut shared_tweet_id: Option<String> = None;
    let mut shared_tweet_url: Option<String> = None;

    // Read top-level fields
    loop {
        let Some(field_type) = reader.read_byte() else { break };
        if field_type == THRIFT_STOP {
            break;
        }
        let Some(field_id) = reader.read_i16() else { break };

        match (field_type, field_id) {
            (THRIFT_STRING, 1) => { message_id = reader.read_string()?; }
            (THRIFT_STRING, 2) => { uuid = reader.read_string()?; }
            (THRIFT_STRING, 3) => { sender_id = reader.read_string()?; }
            (THRIFT_STRING, 4) => { conversation_id = reader.read_string()?; }
            (THRIFT_STRING, 5) => { jwt_token = reader.read_string().map(Some)?; }
            (THRIFT_STRING, 6) => { timestamp_ms = reader.read_string()?; }
            (THRIFT_STRUCT, 7) => {
                // Message content struct — parse recursively for text
                let result = parse_message_content(&mut reader);
                text = result.text;
                shared_tweet_id = result.tweet_id;
                shared_tweet_url = result.tweet_url;
            }
            _ => {
                reader.skip(field_type);
            }
        }
    }

    Some(XChatMessageEvent {
        message_id,
        sender_id,
        conversation_id,
        timestamp_ms,
        text,
        shared_tweet_id,
        shared_tweet_url,
        is_encrypted: jwt_token.is_some(),
    })
}

struct MessageContent {
    text: Option<String>,
    tweet_id: Option<String>,
    tweet_url: Option<String>,
}

/// Parse the nested message content struct to extract text and shared tweet info.
fn parse_message_content(reader: &mut ThriftReader) -> MessageContent {
    let mut result = MessageContent {
        text: None,
        tweet_id: None,
        tweet_url: None,
    };

    // The content is a nested struct. We need to walk it looking for
    // readable text strings and tweet URLs.
    // Structure observed:
    //   field 7 (struct) -> field 1 (struct) -> ... contains text
    //   Shared tweets appear as URLs like "https://x.com/i/status/..."
    let start_pos = reader.pos;
    let mut depth = 0;
    let mut all_strings: Vec<String> = Vec::new();

    // Simple recursive string extraction
    fn extract_strings(reader: &mut ThriftReader, strings: &mut Vec<String>, max_depth: u32) {
        if max_depth == 0 { return; }
        loop {
            let Some(ft) = reader.read_byte() else { return };
            if ft == THRIFT_STOP { return; }
            let Some(_fid) = reader.read_i16() else { return };

            match ft {
                THRIFT_STRING => {
                    if let Some(s) = reader.read_string() {
                        strings.push(s);
                    } else {
                        return;
                    }
                }
                THRIFT_STRUCT => {
                    extract_strings(reader, strings, max_depth - 1);
                }
                THRIFT_LIST => {
                    let Some(elem_type) = reader.read_byte() else { return };
                    let Some(count) = reader.read_i32() else { return };
                    for _ in 0..count.min(100) {
                        if elem_type == THRIFT_STRUCT {
                            extract_strings(reader, strings, max_depth - 1);
                        } else if elem_type == THRIFT_STRING {
                            if let Some(s) = reader.read_string() {
                                strings.push(s);
                            }
                        } else {
                            let _ = reader.skip(elem_type);
                        }
                    }
                }
                _ => {
                    if reader.skip(ft).is_none() { return; }
                }
            }
        }
    }

    extract_strings(reader, &mut all_strings, 10);

    // Find text and tweet URL from extracted strings
    for s in &all_strings {
        if s.starts_with("https://x.com/") && s.contains("/status/") {
            result.tweet_url = Some(s.clone());
            // Extract tweet ID from URL
            if let Some(id) = s.rsplit('/').next() {
                // Strip query params
                let id = id.split('?').next().unwrap_or(id);
                result.tweet_id = Some(id.to_string());
            }
        }
    }

    // The first non-URL, non-ID string that looks like message text
    for s in &all_strings {
        if !s.starts_with("https://")
            && !s.starts_with("http://")
            && !s.chars().all(|c| c.is_ascii_digit())
            && s.len() > 0
            && s.len() < 10000
        {
            result.text = Some(s.clone());
            break;
        }
    }

    // If no plain text found but there's a tweet URL, use that as the text
    if result.text.is_none() {
        if let Some(ref url) = result.tweet_url {
            result.text = Some(url.clone());
        }
    }

    result
}

/// Parse the XChat inbox GraphQL response into our DM models.
pub fn parse_xchat_inbox(data: &Value) -> (Vec<DmConversation>, std::collections::HashMap<String, Vec<DmMessage>>) {
    let mut conversations = Vec::new();
    let mut messages_map: std::collections::HashMap<String, Vec<DmMessage>> = std::collections::HashMap::new();

    let Some(page) = data
        .get("data")
        .and_then(|d| d.get("get_initial_chat_page"))
    else {
        return (conversations, messages_map);
    };

    let Some(items) = page.get("items").and_then(|i| i.as_array()) else {
        return (conversations, messages_map);
    };

    for item in items {
        let Some(detail) = item.get("conversation_detail") else { continue };
        let conv_id = detail
            .get("conversation_id")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();

        // Skip self-conversation
        let parts: Vec<&str> = conv_id.split(':').collect();
        if parts.len() == 2 && parts[0] == parts[1] {
            continue;
        }

        // Get participant info
        let participant = detail
            .get("participants_results")
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .map(|p| {
                let result = p.get("result");
                let core = result.and_then(|r| r.get("core"));
                User {
                    id: p.get("rest_id").and_then(|r| r.as_str()).unwrap_or_default().to_string(),
                    name: core.and_then(|c| c.get("name")).and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                    screen_name: core.and_then(|c| c.get("screen_name")).and_then(|s| s.as_str()).unwrap_or_default().to_string(),
                    description: None,
                    followers_count: 0,
                    following_count: 0,
                    tweet_count: 0,
                    verified: result
                        .and_then(|r| r.get("verification"))
                        .and_then(|v| v.get("is_blue_verified"))
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false),
                    profile_image_url: result
                        .and_then(|r| r.get("avatar"))
                        .and_then(|a| a.get("image_url"))
                        .and_then(|u| u.as_str())
                        .map(String::from),
                    profile_banner_url: None,
                    created_at: None,
                    following: false,
                    followed_by: false,
                }
            })
            .unwrap_or_else(|| User {
                id: String::new(),
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
            });

        // Decode message events
        let mut msgs = Vec::new();
        if let Some(events) = item.get("latest_message_events").and_then(|e| e.as_array()) {
            for event_b64 in events {
                if let Some(b64_str) = event_b64.as_str() {
                    if let Some(decoded) = decode_message_event(b64_str) {
                        let text = if decoded.is_encrypted {
                            decoded.text.unwrap_or_else(|| "[Encrypted message]".to_string())
                        } else {
                            decoded.text.unwrap_or_default()
                        };

                        let ts_ms: Option<i64> = decoded.timestamp_ms.parse().ok();
                        let created_at = ts_ms.and_then(chrono::DateTime::from_timestamp_millis);

                        msgs.push(DmMessage {
                            id: decoded.message_id,
                            text,
                            sender_id: decoded.sender_id,
                            created_at,
                        });
                    }
                }
            }
        }

        // Sort messages chronologically
        msgs.sort_by(|a, b| a.id.cmp(&b.id));

        let last_message = msgs.last().cloned();
        let unread = false; // TODO: parse read events

        messages_map.insert(conv_id.clone(), msgs);

        conversations.push(DmConversation {
            id: conv_id,
            participant,
            last_message,
            unread,
        });
    }

    // Sort conversations by most recent message
    conversations.sort_by(|a, b| {
        let a_id = a.last_message.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        let b_id = b.last_message.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        b_id.cmp(a_id)
    });

    (conversations, messages_map)
}
