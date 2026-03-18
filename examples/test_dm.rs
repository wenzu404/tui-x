use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let auth_path = dirs::config_dir().unwrap().join("tui-x").join("auth.json");
    let store: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&auth_path)?)?;
    let account = &store["accounts"]["main"];
    let auth_token = account["auth_token"].as_str().unwrap();
    let ct0 = account["ct0"].as_str().unwrap();

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
        .build()?;

    let bearer = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

    // Test 1: DM inbox via v1.1
    println!("=== Test 1: DM Inbox (v1.1) ===");
    let resp = client
        .get("https://x.com/i/api/1.1/dm/inbox_initial_state.json")
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-csrf-token", ct0)
        .header("x-twitter-auth-type", "OAuth2Session")
        .header("x-twitter-active-user", "yes")
        .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
        .header("referer", "https://x.com/messages")
        .query(&[
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
        ])
        .send()
        .await?;

    let status = resp.status();
    println!("Status: {status}");
    let headers = resp.headers().clone();
    let body: serde_json::Value = resp.json().await?;

    // Check for errors
    if let Some(errors) = body.get("errors") {
        println!("ERRORS: {}", serde_json::to_string_pretty(errors)?);
    }

    // Print top-level keys
    if let Some(obj) = body.as_object() {
        println!("Top-level keys: {:?}", obj.keys().collect::<Vec<_>>());
    }

    // Print first 3000 chars
    let pretty = serde_json::to_string_pretty(&body)?;
    println!("{}", &pretty[..pretty.len().min(3000)]);

    // Test 2: Try the newer DM API v2
    println!("\n\n=== Test 2: DM via GraphQL (dm_inbox_timeline) ===");

    // Check if there's a GraphQL operation for DMs
    let cache_path = dirs::cache_dir().unwrap().join("tui-x").join("graphql_ops.json");
    if cache_path.exists() {
        let ops: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
        if let Some(ops_map) = ops.get("operations").and_then(|o| o.as_object()) {
            let dm_ops: Vec<_> = ops_map.keys()
                .filter(|k| k.to_lowercase().contains("dm") || k.to_lowercase().contains("message") || k.to_lowercase().contains("inbox"))
                .collect();
            println!("DM-related GraphQL ops: {:?}", dm_ops);

            for op_name in &dm_ops {
                if let Some(op) = ops_map.get(*op_name) {
                    println!("  {} -> queryId: {}", op_name, op.get("query_id").and_then(|q| q.as_str()).unwrap_or("?"));
                }
            }
        }
    }

    // Test 3: Try the v1.1 dm/user_updates endpoint
    println!("\n\n=== Test 3: DM user_updates (v1.1) ===");
    let resp3 = client
        .get("https://x.com/i/api/1.1/dm/user_updates.json")
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-csrf-token", ct0)
        .header("x-twitter-auth-type", "OAuth2Session")
        .header("x-twitter-active-user", "yes")
        .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
        .header("referer", "https://x.com/messages")
        .query(&[
            ("nsfw_filtering_enabled", "false"),
            ("filter_low_quality", "false"),
            ("include_quality", "all"),
            ("dm_secret_conversations_enabled", "false"),
            ("krs_registration_enabled", "true"),
            ("cards_platform", "Web-12"),
            ("include_cards", "1"),
            ("include_ext_alt_text", "true"),
            ("tweet_mode", "extended"),
        ])
        .send()
        .await?;

    let status3 = resp3.status();
    println!("Status: {status3}");
    let body3: serde_json::Value = resp3.json().await?;
    let pretty3 = serde_json::to_string_pretty(&body3)?;
    println!("{}", &pretty3[..pretty3.len().min(2000)]);

    Ok(())
}
