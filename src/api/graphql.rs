use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::config::Config;

const GRAPHQL_OPS_CACHE_FILE: &str = "graphql_ops.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

/// A cached GraphQL operation with its query ID and feature switches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLOperation {
    pub query_id: String,
    pub operation_name: String,
    #[serde(default)]
    pub features: Vec<String>,
}

/// Cache of all extracted GraphQL operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLOpsCache {
    pub operations: HashMap<String, GraphQLOperation>,
    pub fetched_at: u64, // unix timestamp
}

/// Fallback query IDs for operations known to be removed from JS bundles.
pub fn fallback_query_ids() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("SearchTimeline", "nK1dw4oV3k4w5TdtcAdSww"),
        ("CreateRetweet", "ojPdsZsimiJrUGLR1sjUtA"),
        ("DeleteRetweet", "iQtK4dl5hBmXewYZuEOKVw"),
        ("CreateBookmark", "aoDbu3RHznuiSkQ9aNM67Q"),
        ("DeleteBookmark", "Wlmlj2-xzyS1GN3a6cj-mQ"),
    ])
}

/// Hardcoded query IDs for operations not in JS bundles.
pub fn hardcoded_query_ids() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("CreateScheduledTweet", "LCVzRQGxOaGnOnYH01NQXg"),
        ("FetchScheduledTweets", "ITtjAzvlZni2wWXwf295Qg"),
        ("DeleteScheduledTweet", "CTOVqej0JBXAZSwkp1US0g"),
    ])
}

impl GraphQLOpsCache {
    fn cache_path() -> PathBuf {
        Config::cache_dir().join(GRAPHQL_OPS_CACHE_FILE)
    }

    /// Load from disk if cache is still valid.
    pub fn load_cached() -> Option<Self> {
        let path = Self::cache_path();
        let content = std::fs::read_to_string(path).ok()?;
        let cache: Self = serde_json::from_str(&content).ok()?;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now - cache.fetched_at < CACHE_TTL.as_secs() {
            Some(cache)
        } else {
            None
        }
    }

    /// Save to disk.
    pub fn save(&self) -> Result<()> {
        let dir = Config::cache_dir();
        std::fs::create_dir_all(&dir)?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::cache_path(), content)?;
        Ok(())
    }

    /// Invalidate the cache (force re-fetch on next use).
    pub fn invalidate() {
        let _ = std::fs::remove_file(Self::cache_path());
    }

    pub fn get(&self, operation_name: &str) -> Option<&GraphQLOperation> {
        self.operations.get(operation_name)
    }
}

/// Extract GraphQL operations from X.com's JS bundles.
pub async fn extract_operations(client: &reqwest::Client) -> Result<GraphQLOpsCache> {
    tracing::info!("Extracting GraphQL operations from X.com JS bundles...");

    // Step 1: Fetch x.com main page to find JS bundle URLs
    let html = client
        .get("https://x.com")
        .send()
        .await?
        .text()
        .await
        .context("Failed to fetch x.com HTML")?;

    // Step 2: Extract JS bundle URLs
    let bundle_re = Regex::new(
        r#"href="(https://abs\.twimg\.com/responsive-web/client-web[^"]+\.js)""#
    )?;

    let bundle_urls: Vec<String> = bundle_re
        .captures_iter(&html)
        .map(|c| c[1].to_string())
        .collect();

    tracing::debug!("Found {} JS bundle URLs", bundle_urls.len());

    if bundle_urls.is_empty() {
        anyhow::bail!("No JS bundle URLs found on x.com - the page structure may have changed");
    }

    // Step 3: Fetch bundles and extract operations
    let mut operations = HashMap::new();
    let op_re = Regex::new(
        r#"queryId:\s*"([A-Za-z0-9_-]+)".*?operationName:\s*"([A-Za-z]+)""#
    )?;
    let features_re = Regex::new(r#"featureSwitches:\s*(\[[^\]]*\])"#)?;

    for url in &bundle_urls {
        let Ok(resp) = client.get(url).send().await else {
            continue;
        };
        let Ok(js) = resp.text().await else {
            continue;
        };

        // Find all operations in this bundle
        for cap in op_re.captures_iter(&js) {
            let query_id = cap[1].to_string();
            let operation_name = cap[2].to_string();

            // Try to extract feature switches for this operation
            // (they appear near the operation definition)
            let features = if let Some(feat_cap) = features_re
                .captures_iter(&js)
                .next()
            {
                serde_json::from_str::<Vec<String>>(&feat_cap[1]).unwrap_or_default()
            } else {
                Vec::new()
            };

            operations.insert(
                operation_name.clone(),
                GraphQLOperation {
                    query_id,
                    operation_name,
                    features,
                },
            );
        }
    }

    // Add hardcoded operations
    for (name, id) in hardcoded_query_ids() {
        operations.entry(name.to_string()).or_insert_with(|| {
            GraphQLOperation {
                query_id: id.to_string(),
                operation_name: name.to_string(),
                features: Vec::new(),
            }
        });
    }

    tracing::info!("Extracted {} GraphQL operations", operations.len());

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let cache = GraphQLOpsCache {
        operations,
        fetched_at: now,
    };

    cache.save()?;
    Ok(cache)
}
