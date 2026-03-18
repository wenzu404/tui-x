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

    // Page 1
    println!("=== Page 1 (initial) ===");
    let data = client.dm_inbox(None).await?;
    let inbox = data.get("inbox_initial_state").unwrap();

    let convos = inbox.get("conversations").and_then(|c| c.as_object());
    let n = convos.map(|c| c.len()).unwrap_or(0);
    println!("Conversations: {n}");

    let cursor = inbox.get("cursor").and_then(|c| c.as_str());
    println!("Cursor: {:?}", cursor.map(|c| &c[..c.len().min(40)]));

    if let Some(users) = inbox.get("users").and_then(|u| u.as_object()) {
        for (id, user) in users {
            let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
            println!("  user {id}: {name} (@{sn})");
        }
    }

    // Page 2 (if cursor exists)
    if let Some(cursor) = cursor {
        println!("\n=== Page 2 (paginated) ===");
        match client.dm_inbox(Some(cursor)).await {
            Ok(data2) => {
                // Check which key the response uses
                let keys: Vec<_> = data2.as_object().map(|o| o.keys().collect()).unwrap_or_default();
                println!("Response keys: {:?}", keys);

                let inbox2 = data2.get("inbox_timeline")
                    .or_else(|| data2.get("inbox_initial_state"));

                if let Some(inbox2) = inbox2 {
                    let convos2 = inbox2.get("conversations").and_then(|c| c.as_object());
                    let n2 = convos2.map(|c| c.len()).unwrap_or(0);
                    println!("Conversations: {n2}");

                    if let Some(users2) = inbox2.get("users").and_then(|u| u.as_object()) {
                        for (id, user) in users2 {
                            let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let sn = user.get("screen_name").and_then(|s| s.as_str()).unwrap_or("?");
                            println!("  user {id}: {name} (@{sn})");
                        }
                    }

                    let cursor2 = inbox2.get("cursor").and_then(|c| c.as_str());
                    println!("Next cursor: {:?}", cursor2.map(|c| &c[..c.len().min(40)]));

                    let entries2 = inbox2.get("entries").and_then(|e| e.as_array());
                    println!("Entries: {}", entries2.map(|e| e.len()).unwrap_or(0));
                } else {
                    let pretty = serde_json::to_string_pretty(&data2)?;
                    println!("No inbox found. Full response:\n{}", &pretty[..pretty.len().min(2000)]);
                }
            }
            Err(e) => println!("Error: {e}"),
        }
    }

    println!("\nDone!");
    Ok(())
}
