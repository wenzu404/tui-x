use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_read_delay")]
    pub read_delay_ms: u64,
    #[serde(default = "default_write_delay_min")]
    pub write_delay_min_ms: u64,
    #[serde(default = "default_write_delay_max")]
    pub write_delay_max_ms: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub proxy: Option<String>,
}

fn default_read_delay() -> u64 { 1500 }
fn default_write_delay_min() -> u64 { 1500 }
fn default_write_delay_max() -> u64 { 4000 }
fn default_max_retries() -> u32 { 3 }

impl Default for Config {
    fn default() -> Self {
        Self {
            read_delay_ms: default_read_delay(),
            write_delay_min_ms: default_write_delay_min(),
            write_delay_max_ms: default_write_delay_max(),
            max_retries: default_max_retries(),
            proxy: None,
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("tui-x")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_dir().join("config.toml");
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(dir.join("config.toml"), content)?;
        Ok(())
    }

    pub fn cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("~/.cache"))
            .join("tui-x")
    }
}
