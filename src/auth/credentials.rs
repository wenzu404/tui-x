use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub auth_token: String,
    pub ct0: String,
    #[serde(default)]
    pub account_name: Option<String>,
}

impl Credentials {
    pub fn new(auth_token: String, ct0: String) -> Self {
        Self {
            auth_token,
            ct0,
            account_name: None,
        }
    }

    /// Try to load credentials from environment variables.
    pub fn from_env() -> Option<Self> {
        let auth_token = std::env::var("X_AUTH_TOKEN")
            .or_else(|_| std::env::var("TWITTER_AUTH_TOKEN"))
            .ok()?;
        let ct0 = std::env::var("X_CT0")
            .or_else(|_| std::env::var("TWITTER_CT0"))
            .ok()?;
        Some(Self::new(auth_token, ct0))
    }
}
