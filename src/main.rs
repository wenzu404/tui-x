mod api;
mod app;
mod auth;
mod config;
mod tui;

use anyhow::Result;
use app::App;

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

    let mut app = App::new().await?;
    app.run().await
}
