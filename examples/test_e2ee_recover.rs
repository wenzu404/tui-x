use anyhow::Result;
use tui_x::api::juicebox::{self, JuiceboxConfig};
use tui_x::api::XClient;
use tui_x::auth::AuthStore;
use tui_x::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("tui_x=debug,info")
        .init();

    let config = Config::load().unwrap_or_default();
    let store = AuthStore::load()?;
    let creds = store.resolve_credentials().expect("No credentials");
    let client = XClient::new(creds, config).await?;

    // Step 1: Fetch public keys to get Juicebox config
    println!("Fetching public keys...");
    let my_user_id = "2027410882960261120"; // @elstvr

    let pk_data = client.get_public_keys(&[my_user_id]).await?;
    let jb_config = JuiceboxConfig::from_public_keys_response(&pk_data, my_user_id)
        .expect("Failed to parse Juicebox config");

    println!("Juicebox config:");
    println!("  {} realms, threshold {}", jb_config.realms.len(), jb_config.recover_threshold);
    for (i, r) in jb_config.realms.iter().enumerate() {
        println!("  realm {}: {} (hw={})", i + 1, r.address, r.public_key.is_some());
    }

    // Step 2: Ask for PIN
    println!("\nEnter your 4-digit PIN:");
    let mut pin = String::new();
    std::io::stdin().read_line(&mut pin)?;
    let pin = pin.trim();
    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        println!("Invalid PIN. Must be 4 digits.");
        return Ok(());
    }

    // Step 3: Recover private key
    println!("\nRecovering private key (this may take a moment for Argon2)...");
    let http = reqwest::Client::new();
    let secret = juicebox::recover_private_key(&http, &jb_config, pin, my_user_id).await?;

    println!("\nRecovered secret: {} bytes", secret.len());
    println!("First 16 bytes (hex): {}", secret.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>());

    println!("\nDone!");
    Ok(())
}
