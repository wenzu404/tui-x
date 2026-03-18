mod api;
mod app;
mod auth;
mod config;
mod tui;

use anyhow::Result;
use app::App;
use ratatui_image::picker::{Picker, ProtocolType};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to file (don't pollute TUI)
    tracing_subscriber::fmt()
        .with_env_filter("tui_x=debug")
        .with_writer(|| {
            let log_dir = dirs::data_local_dir()
                .unwrap_or_default()
                .join("tui-x");
            std::fs::create_dir_all(&log_dir).ok();
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("tui-x.log"))
                .unwrap()
        })
        .init();

    // Create image picker BEFORE entering alternate screen
    // (it queries terminal capabilities via stdio)
    let picker = Picker::from_query_stdio()
        .unwrap_or_else(|_| {
            let mut p = Picker::halfblocks();
            p.set_protocol_type(ProtocolType::Kitty);
            p
        });

    let mut app = App::new(picker).await?;
    app.run().await
}
