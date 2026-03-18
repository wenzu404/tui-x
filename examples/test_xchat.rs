use anyhow::Result;
use tui_x::api::XClient;
use tui_x::api::xchat;
use tui_x::auth::AuthStore;
use tui_x::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let store = AuthStore::load()?;
    let creds = store.resolve_credentials().expect("No credentials");
    let client = XClient::new(creds, config).await?;

    println!("Fetching XChat inbox...");
    let data = client.xchat_inbox().await?;

    let (conversations, messages) = xchat::parse_xchat_inbox(&data);

    println!("\n{} conversations:", conversations.len());
    for conv in &conversations {
        println!(
            "\n  {} (@{}) — conv_id: {}",
            conv.participant.name, conv.participant.screen_name, conv.id
        );
        if let Some(ref msg) = conv.last_message {
            let preview: String = msg.text.chars().take(60).collect();
            let time = msg.created_at.map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
            println!("    last: [{time}] {preview}");
        }

        // Show all messages in this conversation
        if let Some(msgs) = messages.get(&conv.id) {
            println!("    {} messages:", msgs.len());
            for msg in msgs {
                let time = msg.created_at.map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
                let preview: String = msg.text.chars().take(80).collect();
                let who = if msg.sender_id == conv.participant.id {
                    &conv.participant.screen_name
                } else {
                    "me"
                };
                println!("      [{time}] {who}: {preview}");
            }
        }
    }

    println!("\nDone!");
    Ok(())
}
