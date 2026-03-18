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

    let tl = client.home_latest(10, None).await?;

    for tweet in &tl.tweets {
        if !tweet.media.is_empty() {
            println!("@{} — {} media:", tweet.author.screen_name, tweet.media.len());
            for m in &tweet.media {
                println!("  type: {:?}", m.media_type);
                println!("  url: {}", m.url);
                println!("  thumbnail: {:?}", m.thumbnail_url);
                println!("  size: {:?}x{:?}", m.width, m.height);
            }
            println!();
        }
    }

    let with_media = tl.tweets.iter().filter(|t| !t.media.is_empty()).count();
    let total = tl.tweets.len();
    println!("{with_media}/{total} tweets have media");
    Ok(())
}
