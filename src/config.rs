use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const CONFIG_FILE_PATH: &str = "config.json";
pub const SECRETS_FILE_PATH: &str = "secrets.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub museum_id: String,
    pub ticket_type: String,
    pub shop_domain: String,
    pub calendar_url: String,
    pub quotas_url: String,
    pub booking_page_url: String,
    pub calendar_months: Vec<String>,
    pub date_start: String,
    pub date_end: String,
    pub excluded_dates: Vec<String>,
    /// Minimum available tickets required to trigger an alert (inclusive, `capacity >= min_tickets`).
    pub min_tickets: i64,
    /// Morning slots strictly before this hour (`hour < morning_before_hour`) are eligible.
    pub morning_before_hour: u32,
    /// Afternoon slots from this hour onwards (`hour >= afternoon_from_hour`) are eligible.
    pub afternoon_from_hour: u32,
    pub base_interval_secs: u64,
    pub max_jitter_secs: u64,
    pub max_recent_checks: usize,
    pub state_file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsConfig {
    #[serde(default = "default_ntfy_base_url")]
    pub ntfy_base_url: String,
    pub ntfy_topic: String,
}

fn default_ntfy_base_url() -> String {
    "https://ntfy.sh".to_string()
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let cfg: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(cfg)
}

pub fn load_secrets(path: &Path) -> Result<SecretsConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read secrets file {}", path.display()))?;
    let mut secrets: SecretsConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse secrets file {}", path.display()))?;

    // Allow environment variable override if present.
    if let Ok(env_topic) = std::env::var("NTFY_TOPIC") {
        let trimmed = env_topic.trim();
        if !trimmed.is_empty() {
            secrets.ntfy_topic = trimmed.to_string();
        }
    }

    if secrets.ntfy_topic.trim().is_empty() {
        bail!("ntfy_topic in {} must not be empty", path.display());
    }

    Ok(secrets)
}
