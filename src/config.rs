use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_PATH: &str = "config/config.json";
pub const FILTERS_FILE_PATH: &str = "config/filters.json";
pub const SECRETS_FILE_PATH: &str = "config/secrets.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub ticket_type: String,
    pub shop_domain: String,
    pub calendar_url: String,
    pub quotas_url: String,
    pub booking_page_url: String,
    pub min_poll_interval_secs: u64,
    pub max_poll_interval_secs: u64,
    pub max_recent_checks: usize,
    pub state_file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    pub museum_id: String,
    pub calendar_months: Vec<String>,
    pub date_start: String,
    pub date_end: String,
    pub excluded_dates: Vec<String>,
    pub min_available_tickets: i64,
    pub morning_before_hour: u32,
    pub afternoon_from_hour: u32,
}

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
    pub min_available_tickets: i64,
    pub morning_before_hour: u32,
    pub afternoon_from_hour: u32,
    pub min_poll_interval_secs: u64,
    pub max_poll_interval_secs: u64,
    pub max_recent_checks: usize,
    pub state_file_path: String,
}

impl AppConfig {
    pub fn from_system_and_filter_configs(system: SystemConfig, filters: FilterConfig) -> Self {
        let base_booking_url = system.booking_page_url.trim_end_matches('/');
        let booking_page_url = format!("{base_booking_url}/?museum_id={}", filters.museum_id);

        Self {
            museum_id: filters.museum_id,
            ticket_type: system.ticket_type,
            shop_domain: system.shop_domain,
            calendar_url: system.calendar_url,
            quotas_url: system.quotas_url,
            booking_page_url,
            calendar_months: filters.calendar_months,
            date_start: filters.date_start,
            date_end: filters.date_end,
            excluded_dates: filters.excluded_dates,
            min_available_tickets: filters.min_available_tickets,
            morning_before_hour: filters.morning_before_hour,
            afternoon_from_hour: filters.afternoon_from_hour,
            min_poll_interval_secs: system.min_poll_interval_secs,
            max_poll_interval_secs: system.max_poll_interval_secs,
            max_recent_checks: system.max_recent_checks,
            state_file_path: system.state_file_path,
        }
    }
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

pub trait ConfigurationSource {
    fn load_app_config(&self) -> Result<AppConfig>;
    fn load_secrets_config(&self) -> Result<SecretsConfig>;
}

pub struct JsonFileConfigurationSource {
    config_path: PathBuf,
    filters_path: PathBuf,
    secrets_path: PathBuf,
}

impl JsonFileConfigurationSource {
    pub fn new(
        config_path: impl Into<PathBuf>,
        filters_path: impl Into<PathBuf>,
        secrets_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_path: config_path.into(),
            filters_path: filters_path.into(),
            secrets_path: secrets_path.into(),
        }
    }
}

impl ConfigurationSource for JsonFileConfigurationSource {
    fn load_app_config(&self) -> Result<AppConfig> {
        let system = read_system_config_from_path(&self.config_path)?;
        let filters = read_filter_config_from_path(&self.filters_path)?;
        Ok(AppConfig::from_system_and_filter_configs(system, filters))
    }

    fn load_secrets_config(&self) -> Result<SecretsConfig> {
        read_secrets_config_from_path(&self.secrets_path)
    }
}

pub trait TicketSlotFilter {
    fn is_date_within_target_window(&self, date: &str) -> bool;
    fn is_timestamp_within_desired_hours(&self, timestamp: &str) -> bool;
    fn is_slot_eligible_for_notification(
        &self,
        timestamp: &str,
        capacity: i64,
        already_notified_slots: &BTreeSet<String>,
    ) -> bool;
}

impl TicketSlotFilter for AppConfig {
    fn is_date_within_target_window(&self, date: &str) -> bool {
        date >= self.date_start.as_str()
            && date <= self.date_end.as_str()
            && !self.excluded_dates.iter().any(|excluded| excluded == date)
    }

    fn is_timestamp_within_desired_hours(&self, timestamp: &str) -> bool {
        let Some((hour, minute)) = extract_hour_and_minute_from_iso8601(timestamp) else {
            return false;
        };
        let is_within_morning_window =
            hour < self.morning_before_hour || (hour == self.morning_before_hour && minute == 0);
        let is_within_afternoon_window = hour >= self.afternoon_from_hour;

        is_within_morning_window || is_within_afternoon_window
    }

    fn is_slot_eligible_for_notification(
        &self,
        timestamp: &str,
        capacity: i64,
        already_notified_slots: &BTreeSet<String>,
    ) -> bool {
        capacity >= self.min_available_tickets
            && self.is_timestamp_within_desired_hours(timestamp)
            && !already_notified_slots.contains(timestamp)
    }
}

pub fn extract_hour_and_minute_from_iso8601(timestamp: &str) -> Option<(u32, u32)> {
    let (_, time_part) = timestamp.split_once('T')?;
    if time_part.len() < 5 || time_part.as_bytes()[2] != b':' {
        return None;
    }
    let hour: u32 = time_part[0..2].parse().ok()?;
    let minute: u32 = time_part[3..5].parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

fn read_system_config_from_path(path: &Path) -> Result<SystemConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read system config file {}", path.display()))?;
    let cfg: SystemConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse system config file {}", path.display()))?;
    Ok(cfg)
}

fn read_filter_config_from_path(path: &Path) -> Result<FilterConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read filter config file {}", path.display()))?;
    let cfg: FilterConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse filter config file {}", path.display()))?;
    Ok(cfg)
}

fn read_secrets_config_from_path(path: &Path) -> Result<SecretsConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read secrets file {}", path.display()))?;
    let mut secrets: SecretsConfig = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse secrets file {}", path.display()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppConfig {
        let system = SystemConfig {
            ticket_type: "time_slot".to_string(),
            shop_domain: "tickets.jmberlin.de".to_string(),
            calendar_url: "https://jmb.gomus.de/api/v4/tickets/calendar".to_string(),
            quotas_url: "https://jmb.gomus.de/api/v4/tickets/list_and_quotas".to_string(),
            booking_page_url: "https://tickets.jmberlin.de/tickets/".to_string(),
            min_poll_interval_secs: 300,
            max_poll_interval_secs: 600,
            max_recent_checks: 3,
            state_file_path: "data/last_seen_slots.json".to_string(),
        };
        let filters = FilterConfig {
            museum_id: "2".to_string(),
            calendar_months: vec!["2026-12-01".to_string(), "2027-01-01".to_string()],
            date_start: "2026-12-24".to_string(),
            date_end: "2027-01-03".to_string(),
            excluded_dates: vec!["2026-12-24".to_string()],
            min_available_tickets: 2,
            morning_before_hour: 10,
            afternoon_from_hour: 15,
        };
        AppConfig::from_system_and_filter_configs(system, filters)
    }

    #[test]
    fn builds_app_config_and_appends_museum_id_to_booking_url() {
        let cfg = sample_config();
        assert_eq!(cfg.museum_id, "2");
        assert_eq!(
            cfg.booking_page_url,
            "https://tickets.jmberlin.de/tickets/?museum_id=2"
        );
    }

    #[test]
    fn target_date_filtering() {
        let cfg = sample_config();
        assert!(!cfg.is_date_within_target_window("2026-12-23"));
        assert!(!cfg.is_date_within_target_window("2026-12-24"));
        assert!(cfg.is_date_within_target_window("2026-12-25"));
        assert!(cfg.is_date_within_target_window("2026-12-31"));
        assert!(cfg.is_date_within_target_window("2027-01-01"));
        assert!(cfg.is_date_within_target_window("2027-01-03"));
        assert!(!cfg.is_date_within_target_window("2027-01-04"));
    }

    #[test]
    fn desired_time_filtering() {
        let cfg = sample_config();
        assert!(cfg.is_timestamp_within_desired_hours("2026-12-29T09:00:00+01:00"));
        assert!(cfg.is_timestamp_within_desired_hours("2026-12-29T09:45:00+01:00"));
        assert!(cfg.is_timestamp_within_desired_hours("2026-12-29T10:00:00+01:00"));
        assert!(!cfg.is_timestamp_within_desired_hours("2026-12-29T10:15:00+01:00"));
        assert!(!cfg.is_timestamp_within_desired_hours("2026-12-29T14:45:00+01:00"));
        assert!(cfg.is_timestamp_within_desired_hours("2026-12-29T15:00:00+01:00"));
        assert!(cfg.is_timestamp_within_desired_hours("2026-12-29T16:15:00+01:00"));
        assert!(!cfg.is_timestamp_within_desired_hours("invalid-timestamp"));
    }

    #[test]
    fn slot_eligibility_checks_capacity_time_and_notified() {
        let cfg = sample_config();
        let mut notified = BTreeSet::new();
        notified.insert("2026-12-29T09:15:00+01:00".to_string());

        assert!(!cfg.is_slot_eligible_for_notification("2026-12-29T09:30:00+01:00", 1, &notified));
        assert!(cfg.is_slot_eligible_for_notification("2026-12-29T09:30:00+01:00", 2, &notified));
        assert!(!cfg.is_slot_eligible_for_notification("2026-12-29T09:15:00+01:00", 5, &notified));
        assert!(!cfg.is_slot_eligible_for_notification("2026-12-29T11:00:00+01:00", 5, &notified));
    }
}
