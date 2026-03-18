use anyhow::Result;
use tui_x::api::XClient;
use tui_x::auth::AuthStore;
use tui_x::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let store = AuthStore::load()?;
    let creds = store.resolve_credentials().expect("No credentials");
    let client = XClient::new(creds, config).await?;

    // 1. Get hussein's user ID
    println!("=== Looking up @s4_hussein ===");
    let hussein = client.user_by_screen_name("s4_hussein").await?;
    println!("Found: {} (@{}) id={}", hussein.name, hussein.screen_name, hussein.id);

    // 2. Get our own user ID from a known conversation participant
    // We know from inbox: participants are 2027410882960261120 and 2030029147922386944
    // One of those is us. Let's figure out which by checking @elstvr and @shevon_67
    println!("\n=== Checking who we are ===");
    let elstvr = client.user_by_screen_name("elstvr").await;
    let shevon = client.user_by_screen_name("shevon_67").await;
    println!("@elstvr: {:?}", elstvr.as_ref().map(|u| &u.id));
    println!("@shevon_67: {:?}", shevon.as_ref().map(|u| &u.id));

    // 3. Now try to directly access the DM conversation
    // Convention: conversation_id = "{smaller_id}-{larger_id}" for ONE_TO_ONE
    let hussein_id = &hussein.id;
    // We need our ID. From the inbox, one of the participants is us.
    // elstvr=2027410882960261120, shevon_67=2030029147922386944
    // Let's try both as "our" id
    let our_ids = ["2027410882960261120", "2030029147922386944"];

    for our_id in &our_ids {
        let (smaller, larger) = if our_id < &hussein_id.as_str() {
            (*our_id, hussein_id.as_str())
        } else {
            (hussein_id.as_str(), *our_id)
        };
        let conv_id = format!("{smaller}-{larger}");
        println!("\n=== Trying conversation: {conv_id} (our_id={our_id}) ===");

        // Try dm/conversation endpoint
        let auth_path = dirs::config_dir().unwrap().join("tui-x").join("auth.json");
        let store_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&auth_path)?)?;
        let account = &store_json["accounts"]["main"];
        let auth_token = account["auth_token"].as_str().unwrap();
        let ct0 = account["ct0"].as_str().unwrap();

        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
            .build()?;

        let bearer = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

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

        if status.is_success() {
            let data: serde_json::Value = resp.json().await?;
            // Show conversation info
            if let Some(conv_data) = data.get("conversation_timeline") {
                if let Some(convos) = conv_data.get("conversations").and_then(|c| c.as_object()) {
                    println!("Conversations found: {}", convos.len());
                }
                if let Some(entries) = conv_data.get("entries").and_then(|e| e.as_array()) {
                    println!("Messages: {}", entries.len());
                    for entry in entries.iter().take(3) {
                        if let Some(msg) = entry.get("message") {
                            let text = msg.pointer("/message_data/text")
                                .and_then(|t| t.as_str()).unwrap_or("?");
                            let sender = msg.pointer("/message_data/sender_id")
                                .and_then(|s| s.as_str()).unwrap_or("?");
                            let truncated: String = text.chars().take(60).collect();
                            println!("  from={sender}: {truncated}");
                        }
                    }
                }
                if let Some(users) = conv_data.get("users").and_then(|u| u.as_object()) {
                    println!("Users in conv:");
                    for (id, user) in users {
                        let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
                        println!("  {id}: {name} (@{sn})");
                    }
                }
            } else {
                let pretty = serde_json::to_string_pretty(&data)?;
                println!("{}", &pretty[..pretty.len().min(2000)]);
            }
        } else {
            let body = resp.text().await?;
            println!("Error: {}", &body[..body.len().min(500)]);
        }
    }

    println!("\nDone!");
    Ok(())
}
