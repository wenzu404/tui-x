use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let auth_path = dirs::config_dir().unwrap().join("tui-x").join("auth.json");
    let store: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&auth_path)?)?;
    let account = &store["accounts"]["main"];
    let auth_token = account["auth_token"].as_str().unwrap();
    let ct0 = account["ct0"].as_str().unwrap();

    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
        .build()?;

    let bearer = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

    let h = |r: reqwest::RequestBuilder| -> reqwest::RequestBuilder {
        r.header("authorization", format!("Bearer {bearer}"))
            .header("x-csrf-token", ct0)
            .header("x-twitter-auth-type", "OAuth2Session")
            .header("x-twitter-active-user", "yes")
            .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
            .header("referer", "https://x.com/messages")
    };

    // 1. Key registry state
    println!("=== 1. Key Registry State ===");
    let endpoints = [
        "dm/encrypted/keys/query.json",
        "dm/encrypted/keys/status.json",
        "dm/key_registry/state.json",
        "dm/key_registry.json",
    ];
    for ep in &endpoints {
        let url = format!("https://x.com/i/api/1.1/{ep}");
        let resp = h(http.get(&url)).send().await?;
        let status = resp.status();
        print!("  {ep}: {status}");
        if status.is_success() {
            let body: serde_json::Value = resp.json().await?;
            let pretty = serde_json::to_string_pretty(&body)?;
            println!("\n{}", &pretty[..pretty.len().min(1500)]);
        } else {
            let body = resp.text().await?;
            println!(" — {}", &body[..body.len().min(200)]);
        }
    }

    // 2. Fetch the encrypted conversation WITH encryption params
    println!("\n=== 2. Encrypted conv with hussein ===");
    let conv_id = "1836417677612511233-2027410882960261120";
    let url = format!("https://x.com/i/api/1.1/dm/conversation/{conv_id}.json");
    let resp = h(http.get(&url))
        .query(&[
            ("include_profile_interstitial_type", "1"),
            ("include_blocking", "1"),
            ("include_blocked_by", "1"),
            ("include_followed_by", "1"),
            ("include_want_retweets", "1"),
            ("include_mute_edge", "1"),
            ("include_can_dm", "1"),
            ("include_ext_alt_text", "true"),
            ("include_quote_count", "true"),
            ("tweet_mode", "extended"),
            ("dm_secret_conversations_enabled", "true"),
            ("krs_registration_enabled", "true"),
            ("count", "100"),
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl,dmEnc"),
        ])
        .send().await?;
    println!("Status: {}", resp.status());
    let data: serde_json::Value = resp.json().await?;
    let pretty = serde_json::to_string_pretty(&data)?;
    println!("{}", &pretty[..pretty.len().min(3000)]);

    // 3. Check the full inbox with secret convos
    println!("\n=== 3. Inbox with dm_secret_conversations ===");
    let resp = h(http.get("https://x.com/i/api/1.1/dm/inbox_initial_state.json"))
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
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl,dmEnc"),
        ])
        .send().await?;
    println!("Status: {}", resp.status());
    let data: serde_json::Value = resp.json().await?;
    // Look specifically for key_registry_state and encrypted fields
    if let Some(inbox) = data.get("inbox_initial_state") {
        let keys: Vec<_> = inbox.as_object().map(|o| o.keys().collect()).unwrap_or_default();
        println!("Inbox keys: {:?}", keys);

        if let Some(krs) = inbox.get("key_registry_state") {
            println!("\nkey_registry_state:");
            let p = serde_json::to_string_pretty(krs)?;
            println!("{}", &p[..p.len().min(2000)]);
        }

        // Check all conversations for encryption fields
        if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
            for (id, convo) in convos {
                let keys: Vec<_> = convo.as_object().map(|o| o.keys().collect()).unwrap_or_default();
                println!("\nConv {id} keys: {:?}", keys);
            }
        }
    }

    // 4. GraphQL operations related to encryption
    println!("\n=== 4. GraphQL encryption ops ===");
    let cache_path = dirs::cache_dir().unwrap().join("tui-x").join("graphql_ops.json");
    if cache_path.exists() {
        let ops: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
        if let Some(ops_map) = ops.get("operations").and_then(|o| o.as_object()) {
            let enc_ops: Vec<_> = ops_map.keys()
                .filter(|k| {
                    let l = k.to_lowercase();
                    l.contains("encrypt") || l.contains("key") || l.contains("secret")
                        || l.contains("e2ee") || l.contains("pin")
                })
                .collect();
            println!("Encryption-related ops: {:#?}", enc_ops);
            for op_name in &enc_ops {
                if let Some(op) = ops_map.get(*op_name) {
                    println!("  {} -> {}", op_name, op.get("query_id").and_then(|q| q.as_str()).unwrap_or("?"));
                }
            }
        }
    }

    println!("\nDone!");
    Ok(())
}
