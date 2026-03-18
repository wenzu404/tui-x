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

    println!("Fetching DM inbox...");
    let data = client.dm_inbox().await?;

    let inbox = data.get("inbox_initial_state").unwrap();

    // Users
    println!("\n=== Users ===");
    if let Some(users) = inbox.get("users").and_then(|u| u.as_object()) {
        for (id, user) in users {
            let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
            println!("  {id}: {name} (@{sn})");
        }
    } else {
        println!("  No users object found!");
        println!("  inbox keys: {:?}", inbox.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    }

    // Conversations
    println!("\n=== Conversations ===");
    if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
        for (id, convo) in convos {
            println!("  conv_id: {id}");
            println!("    type: {:?}", convo.get("type").and_then(|t| t.as_str()));
            println!("    status: {:?}", convo.get("status").and_then(|s| s.as_str()));
            if let Some(participants) = convo.get("participants").and_then(|p| p.as_array()) {
                for p in participants {
                    println!("    participant: user_id={}", p.get("user_id").and_then(|u| u.as_str()).unwrap_or("?"));
                }
            }
        }
    }

    // Entries (messages)
    println!("\n=== Entries (messages) ===");
    if let Some(entries) = inbox.get("entries").and_then(|e| e.as_array()) {
        println!("  {} entries total", entries.len());
        for entry in entries.iter().take(5) {
            if let Some(msg) = entry.get("message") {
                let conv_id = msg.get("conversation_id").and_then(|c| c.as_str()).unwrap_or("?");
                let msg_id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("?");
                let msg_data = msg.get("message_data");
                let text = msg_data.and_then(|d| d.get("text")).and_then(|t| t.as_str()).unwrap_or("?");
                let sender = msg_data.and_then(|d| d.get("sender_id")).and_then(|s| s.as_str()).unwrap_or("?");
                let time = msg.get("time").and_then(|t| t.as_str()).unwrap_or("?");

                let truncated: String = text.chars().take(80).collect();
                println!("  [{conv_id}] msg_id={msg_id} from={sender} time={time}");
                println!("    text: {truncated}");
            }
        }
    }

    // Check for PIN/encryption related fields
    println!("\n=== Checking for encryption/PIN ===");
    println!("  dm_secret_conversations: {:?}", inbox.get("dm_secret_conversations"));
    println!("  key_registry_state: {:?}", inbox.get("key_registry_state"));
    if let Some(convos) = inbox.get("conversations").and_then(|c| c.as_object()) {
        for (id, convo) in convos {
            println!("  conv {id}:");
            println!("    trusted: {:?}", convo.get("trusted"));
            println!("    nsfw: {:?}", convo.get("nsfw"));
            // Check all keys for anything encryption related
            if let Some(obj) = convo.as_object() {
                let interesting: Vec<_> = obj.keys()
                    .filter(|k| k.contains("encrypt") || k.contains("secret") || k.contains("pin") || k.contains("key"))
                    .collect();
                if !interesting.is_empty() {
                    println!("    encryption-related keys: {:?}", interesting);
                }
            }
        }
    }

    println!("\nDone!");
    Ok(())
}
