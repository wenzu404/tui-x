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

    // Directly fetch the conversation with hussein
    let conv_id = "1836417677612511233-2027410882960261120";
    println!("Fetching conversation: {conv_id}");

    let url = format!("https://x.com/i/api/1.1/dm/conversation/{conv_id}.json");
    let resp = http.get(&url)
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-csrf-token", ct0)
        .header("x-twitter-auth-type", "OAuth2Session")
        .header("x-twitter-active-user", "yes")
        .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
        .header("referer", "https://x.com/messages")
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
            ("ext", "mediaColor,altText,mediaStats,highlightedLabel,hasNftAvatar,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl"),
        ])
        .send().await?;

    let status = resp.status();
    println!("Status: {status}");

    let data: serde_json::Value = resp.json().await?;

    // Print top-level keys
    if let Some(obj) = data.as_object() {
        println!("Top keys: {:?}", obj.keys().collect::<Vec<_>>());
    }

    let pretty = serde_json::to_string_pretty(&data)?;
    println!("{}", &pretty[..pretty.len().min(5000)]);

    Ok(())
}
