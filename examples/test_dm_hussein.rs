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

    let headers = |r: reqwest::RequestBuilder| -> reqwest::RequestBuilder {
        r.header("authorization", format!("Bearer {bearer}"))
            .header("x-csrf-token", ct0)
            .header("x-twitter-auth-type", "OAuth2Session")
            .header("x-twitter-active-user", "yes")
            .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
            .header("referer", "https://x.com/messages")
    };

    // 1. Get my user ID first
    println!("=== Getting my user ID ===");
    let resp = headers(client.get("https://x.com/i/api/1.1/account/verify_credentials.json"))
        .send().await?;
    let me: serde_json::Value = resp.json().await?;
    let my_id = me.get("id_str").and_then(|i| i.as_str()).unwrap_or("?");
    let my_sn = me.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
    println!("I am: @{my_sn} (id: {my_id})");

    // 2. Try inbox with more params + higher count
    println!("\n=== inbox_initial_state with count ===");
    let resp2 = headers(client.get("https://x.com/i/api/1.1/dm/inbox_initial_state.json"))
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
            ("count", "100"),
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl"),
        ])
        .send().await?;
    let status2 = resp2.status();
    println!("Status: {status2}");
    let data2: serde_json::Value = resp2.json().await?;

    if let Some(inbox) = data2.get("inbox_initial_state") {
        if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
            println!("{} conversations:", convos.len());
            for (id, convo) in convos {
                let ctype = convo.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                println!("  {id} (type={ctype})");
            }
        }
        if let Some(users) = inbox.get("users").and_then(|u| u.as_object()) {
            println!("{} users:", users.len());
            for (id, user) in users {
                let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
                println!("  {id}: {name} (@{sn})");
            }
        }

        // Check if there's mention of encryption
        if let Some(key_reg) = inbox.get("key_registry_state") {
            println!("\nKey registry: {}", serde_json::to_string_pretty(key_reg)?);
        }
    }

    // 3. Try dm/search to find hussein
    println!("\n=== DM search for 'hussein' ===");
    let resp3 = headers(client.get("https://x.com/i/api/1.1/dm/search.json"))
        .query(&[("q", "hussein"), ("count", "20")])
        .send().await?;
    let status3 = resp3.status();
    println!("Status: {status3}");
    if status3.is_success() {
        let data3: serde_json::Value = resp3.json().await?;
        let pretty = serde_json::to_string_pretty(&data3)?;
        println!("{}", &pretty[..pretty.len().min(3000)]);
    } else {
        let body = resp3.text().await?;
        println!("Error: {}", &body[..body.len().min(500)]);
    }

    // 4. Look for encrypted DMs / secret conversations
    println!("\n=== Check dm_secret_conversations_enabled=true ===");
    let resp4 = headers(client.get("https://x.com/i/api/1.1/dm/inbox_initial_state.json"))
        .query(&[
            ("nsfw_filtering_enabled", "false"),
            ("filter_low_quality", "false"),
            ("include_quality", "all"),
            ("dm_secret_conversations_enabled", "true"),
            ("krs_registration_enabled", "true"),
            ("cards_platform", "Web-12"),
            ("include_cards", "1"),
            ("include_ext_alt_text", "true"),
            ("tweet_mode", "extended"),
            ("count", "100"),
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl"),
        ])
        .send().await?;
    let status4 = resp4.status();
    println!("Status: {status4}");
    let data4: serde_json::Value = resp4.json().await?;
    if let Some(inbox) = data4.get("inbox_initial_state") {
        if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
            println!("{} conversations:", convos.len());
            for (id, convo) in convos {
                let ctype = convo.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                let trusted = convo.get("trusted").and_then(|t| t.as_bool());
                let enc = convo.get("encryption_enabled").and_then(|e| e.as_bool());
                println!("  {id} (type={ctype}, trusted={trusted:?}, encrypted={enc:?})");
            }
        }
        if let Some(users) = inbox.get("users").and_then(|u| u.as_object()) {
            for (id, user) in users {
                let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
                println!("  {id}: {name} (@{sn})");
            }
        }
    }

    println!("\nDone!");
    Ok(())
}
