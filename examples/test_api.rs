use anyhow::Result;
use tui_x::api::XClient;
use tui_x::auth::AuthStore;
use tui_x::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let store = AuthStore::load()?;
    let creds = store
        .resolve_credentials()
        .expect("No credentials found");

    println!("Connecting...");
    let client = XClient::new(creds, config).await?;

    println!("\n--- Following Timeline (5 tweets) ---\n");
    let tl = client.home_latest(5, None).await?;

    for tweet in &tl.tweets {
        let time = tweet
            .created_at
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default();
        let likes = tweet.like_count;
        let rts = tweet.retweet_count;
        let views = tweet.view_count.unwrap_or(0);
        // Safe truncate at char boundary
        let text: String = tweet.text.chars().take(100).collect();
        let text = text.replace('\n', " ");

        println!("  {} @{} [{}]", tweet.author.name, tweet.author.screen_name, time);
        println!("  {text}");
        println!("  {} likes | {} RTs | {} views", likes, rts, views);
        if !tweet.media.is_empty() {
            println!("  [{} media]", tweet.media.len());
        }
        println!();
    }

    println!("Cursor bottom: {:?}", tl.cursor_bottom.as_deref().map(|c| &c[..c.len().min(40)]));
    println!("\nDone! {} tweets fetched.", tl.tweets.len());
    Ok(())
}
