use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::Credentials;
use crate::config::Config;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub accounts: HashMap<String, Credentials>,
}

impl AuthStore {
    fn path() -> PathBuf {
        Config::config_dir().join("auth.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .context("Failed to read auth.json")?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = Config::config_dir();
        std::fs::create_dir_all(&dir)?;
        let path = Self::path();
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, &content)?;

        // Set restrictive permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Get credentials for the active account.
    pub fn active_credentials(&self) -> Option<&Credentials> {
        // 1. Try env vars first
        // (handled separately since we can't store the result here)

        // 2. Try default account from store
        if let Some(ref name) = self.default {
            return self.accounts.get(name);
        }

        // 3. Try the only account if there's exactly one
        if self.accounts.len() == 1 {
            return self.accounts.values().next();
        }

        None
    }

    pub fn add_account(&mut self, name: String, creds: Credentials) {
        if self.accounts.is_empty() {
            self.default = Some(name.clone());
        }
        self.accounts.insert(name, creds);
    }

    pub fn remove_account(&mut self, name: &str) {
        self.accounts.remove(name);
        if self.default.as_deref() == Some(name) {
            self.default = self.accounts.keys().next().cloned();
        }
    }

    pub fn set_default(&mut self, name: String) {
        self.default = Some(name);
    }

    /// Resolve credentials: env vars take priority, then stored accounts.
    pub fn resolve_credentials(&self) -> Option<Credentials> {
        Credentials::from_env().or_else(|| self.active_credentials().cloned())
    }
}
