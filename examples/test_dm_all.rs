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

    let base_params = vec![
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

    let headers = |r: reqwest::RequestBuilder| -> reqwest::RequestBuilder {
        r.header("authorization", format!("Bearer {bearer}"))
            .header("x-csrf-token", ct0)
            .header("x-twitter-auth-type", "OAuth2Session")
            .header("x-twitter-active-user", "yes")
            .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
            .header("referer", "https://x.com/messages")
    };

    // 1. Standard inbox (trusted)
    println!("=== 1. inbox_initial_state (trusted) ===");
    let resp = headers(client.get("https://x.com/i/api/1.1/dm/inbox_initial_state.json"))
        .query(&base_params)
        .send().await?;
    let data: serde_json::Value = resp.json().await?;
    print_convos(&data, "inbox_initial_state");

    // 2. Untrusted / message requests
    println!("\n=== 2. inbox_timeline/untrusted (message requests) ===");
    let resp2 = headers(client.get("https://x.com/i/api/1.1/dm/inbox_timeline/untrusted.json"))
        .query(&base_params)
        .send().await?;
    let status2 = resp2.status();
    println!("Status: {status2}");
    if status2.is_success() {
        let data2: serde_json::Value = resp2.json().await?;
        print_convos(&data2, "inbox_timeline");
    } else {
        let body = resp2.text().await?;
        println!("Error: {}", &body[..body.len().min(500)]);
    }

    // 3. Try trusted timeline
    println!("\n=== 3. inbox_timeline/trusted ===");
    let resp3 = headers(client.get("https://x.com/i/api/1.1/dm/inbox_timeline/trusted.json"))
        .query(&base_params)
        .send().await?;
    let status3 = resp3.status();
    println!("Status: {status3}");
    if status3.is_success() {
        let data3: serde_json::Value = resp3.json().await?;
        print_convos(&data3, "inbox_timeline");
    } else {
        let body = resp3.text().await?;
        println!("Error: {}", &body[..body.len().min(500)]);
    }

    // 4. Search for "hussein" in GraphQL DM ops
    println!("\n=== 4. GraphQL DM operations ===");
    let cache_path = dirs::cache_dir().unwrap().join("tui-x").join("graphql_ops.json");
    if cache_path.exists() {
        let ops: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
        if let Some(ops_map) = ops.get("operations").and_then(|o| o.as_object()) {
            let dm_ops: Vec<_> = ops_map.keys()
                .filter(|k| {
                    let lower = k.to_lowercase();
                    lower.contains("dm") || lower.contains("message") ||
                    lower.contains("inbox") || lower.contains("conversation")
                })
                .collect();
            println!("DM-related ops: {:#?}", dm_ops);
        }
    }

    // 5. Try DmInboxTimeline GraphQL
    println!("\n=== 5. Trying GraphQL DmInboxTimeline ===");
    let cache_path = dirs::cache_dir().unwrap().join("tui-x").join("graphql_ops.json");
    if cache_path.exists() {
        let ops: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
        // Look for any inbox-related graphql op
        if let Some(ops_map) = ops.get("operations").and_then(|o| o.as_object()) {
            for (name, op) in ops_map {
                let lower = name.to_lowercase();
                if lower.contains("inbox") || lower.contains("dminbox") {
                    let qid = op.get("query_id").and_then(|q| q.as_str()).unwrap_or("?");
                    println!("Trying {name} (queryId: {qid})...");

                    let url = format!("https://x.com/i/api/graphql/{qid}/{name}");
                    let variables = serde_json::json!({
                        "count": 50,
                        "includePromotedContent": false,
                    });
                    let features = serde_json::json!({
                        "responsive_web_graphql_exclude_directive_enabled": true,
                        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
                        "responsive_web_graphql_timeline_navigation_enabled": true,
                    });

                    let resp = headers(client.get(&url))
                        .query(&[
                            ("variables", serde_json::to_string(&variables)?),
                            ("features", serde_json::to_string(&features)?),
                        ])
                        .send().await?;

                    let status = resp.status();
                    println!("  Status: {status}");
                    if status.is_success() {
                        let body: serde_json::Value = resp.json().await?;
                        let pretty = serde_json::to_string_pretty(&body)?;
                        println!("  {}", &pretty[..pretty.len().min(2000)]);
                    }
                }
            }
        }
    }

    println!("\nDone!");
    Ok(())
}

fn print_convos(data: &serde_json::Value, inbox_key: &str) {
    let inbox = data.get(inbox_key);
    if let Some(inbox) = inbox {
        if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
            println!("{} conversations:", convos.len());
            for (id, convo) in convos {
                let ctype = convo.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                let trusted = convo.get("trusted").and_then(|t| t.as_bool());
                println!("  {id} (type={ctype}, trusted={trusted:?})");
            }
        }
        if let Some(users) = inbox.get("users").and_then(|u| u.as_object()) {
            println!("Users:");
            for (id, user) in users {
                let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
                println!("  {id}: {name} (@{sn})");
            }
        }
    } else {
        println!("Key '{inbox_key}' not found. Top-level keys: {:?}",
            data.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    }
}
