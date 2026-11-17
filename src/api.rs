use crate::config::{AppConfig, SecretsConfig};
use crate::models::{AvailableSlot, CalendarResponse, QuotasResponse};
use anyhow::{Context, Result};
use reqwest::Client;
use std::collections::BTreeSet;
use tracing::debug;

/// Queries the calendar endpoint for a given month (`YYYY-MM-01`).
pub async fn fetch_calendar(
    client: &Client,
    cfg: &AppConfig,
    start_at: &str,
) -> Result<CalendarResponse> {
    let response = client
        .get(&cfg.calendar_url)
        .header("X-Shop-Url", &cfg.shop_domain)
        .query(&[
            ("by_bookable", "true"),
            ("by_ticket_types[]", cfg.ticket_type.as_str()),
            ("by_museum_ids[]", cfg.museum_id.as_str()),
            ("start_at", start_at),
        ])
        .send()
        .await
        .with_context(|| format!("calendar request failed for start_at={start_at}"))?;

    let status = response.status();
    debug!(url = %cfg.calendar_url, start_at, %status, "calendar response received");

    response
        .error_for_status()
        .with_context(|| format!("calendar endpoint returned error status for {start_at}"))?
        .json::<CalendarResponse>()
        .await
        .with_context(|| format!("failed to decode calendar JSON for {start_at}"))
}

/// Queries the quota/capacity endpoint for a specific date (`YYYY-MM-DD`).
pub async fn fetch_quotas(
    client: &Client,
    cfg: &AppConfig,
    valid_at: &str,
) -> Result<QuotasResponse> {
    let response = client
        .get(&cfg.quotas_url)
        .header("X-Shop-Url", &cfg.shop_domain)
        .query(&[
            ("by_bookable", "true"),
            ("by_ticket_types[]", cfg.ticket_type.as_str()),
            ("by_museum_ids[]", cfg.museum_id.as_str()),
            ("valid_at", valid_at),
        ])
        .send()
        .await
        .with_context(|| format!("quotas request failed for valid_at={valid_at}"))?;

    let status = response.status();
    debug!(url = %cfg.quotas_url, valid_at, %status, "quotas response received");

    response
        .error_for_status()
        .with_context(|| format!("quotas endpoint returned error status for {valid_at}"))?
        .json::<QuotasResponse>()
        .await
        .with_context(|| format!("failed to decode quotas JSON for {valid_at}"))
}

/// Returns true if `date` (`YYYY-MM-DD`) is within the configured date window and not excluded.
pub fn is_target_date(date: &str, cfg: &AppConfig) -> bool {
    date >= cfg.date_start.as_str()
        && date <= cfg.date_end.as_str()
        && !cfg.excluded_dates.iter().any(|d| d == date)
}

/// Extracts `(hour, minute)` from an ISO-8601 timestamp like `2026-12-29T09:30:00+01:00`.
pub fn parse_hour_minute(timestamp: &str) -> Option<(u32, u32)> {
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

/// Returns true if the slot's local time matches the configured morning or afternoon window.
pub fn is_desired_time(timestamp: &str, cfg: &AppConfig) -> bool {
    let Some((hour, _minute)) = parse_hour_minute(timestamp) else {
        return false;
    };
    hour < cfg.morning_before_hour || hour >= cfg.afternoon_from_hour
}

/// Filters a slot by minimum tickets (`capacity >= cfg.min_tickets`), desired time window, and deduplication set.
pub fn is_eligible_slot(
    timestamp: &str,
    capacity: i64,
    cfg: &AppConfig,
    notified_slots: &BTreeSet<String>,
) -> bool {
    capacity >= cfg.min_tickets
        && is_desired_time(timestamp, cfg)
        && !notified_slots.contains(timestamp)
}

/// Formats a human-readable summary for newly discovered slots.
pub fn format_alert_message(slots: &[AvailableSlot], booking_page_url: &str) -> String {
    let mut lines = Vec::with_capacity(slots.len() + 3);
    lines.push("New availability found:".to_string());
    for slot in slots {
        let display_ts = slot
            .timestamp
            .split_once('+')
            .map(|(prefix, _)| prefix)
            .unwrap_or(&slot.timestamp)
            .replace('T', " ");
        let short_ts = display_ts.get(..16).unwrap_or(&display_ts);
        lines.push(format!("- {short_ts} ({} tickets)", slot.capacity));
    }
    lines.push(format!("Book: {booking_page_url}"));
    lines.join("\n")
}

/// Sends a push notification via ntfy for the newly discovered slots.
pub async fn send_ntfy_alert(
    client: &Client,
    cfg: &AppConfig,
    secrets: &SecretsConfig,
    slots: &[AvailableSlot],
) -> Result<()> {
    let base = secrets.ntfy_base_url.trim_end_matches('/');
    let topic = secrets.ntfy_topic.trim();
    let url = format!("{base}/{topic}");
    let body = format_alert_message(slots, &cfg.booking_page_url);

    let response = client
        .post(&url)
        .header("Title", "JMB Time Slot Alert!")
        .header("Priority", "high")
        .header("Tags", "ticket")
        .header("Click", &cfg.booking_page_url)
        .body(body)
        .send()
        .await
        .with_context(|| format!("failed to send ntfy alert to {base}/<redacted>"))?;

    let status = response.status();
    debug!(%status, "ntfy response received");

    response
        .error_for_status()
        .context("ntfy endpoint returned error status")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            museum_id: "2".to_string(),
            ticket_type: "time_slot".to_string(),
            shop_domain: "tickets.jmberlin.de".to_string(),
            calendar_url: "https://jmb.gomus.de/api/v4/tickets/calendar".to_string(),
            quotas_url: "https://jmb.gomus.de/api/v4/tickets/list_and_quotas".to_string(),
            booking_page_url: "https://tickets.jmberlin.de/en/tickets/?museum_id=2".to_string(),
            calendar_months: vec!["2026-12-01".to_string(), "2027-01-01".to_string()],
            date_start: "2026-12-24".to_string(),
            date_end: "2027-01-03".to_string(),
            excluded_dates: vec!["2026-12-24".to_string()],
            min_tickets: 3,
            morning_before_hour: 10,
            afternoon_from_hour: 15,
            base_interval_secs: 300,
            max_jitter_secs: 300,
            max_recent_checks: 3,
            state_file_path: "last_seen_slots.json".to_string(),
        }
    }

    #[test]
    fn target_date_filtering() {
        let cfg = sample_config();
        assert!(!is_target_date("2026-12-23", &cfg));
        assert!(!is_target_date("2026-12-24", &cfg)); // excluded
        assert!(is_target_date("2026-12-25", &cfg));
        assert!(is_target_date("2026-12-31", &cfg));
        assert!(is_target_date("2027-01-01", &cfg));
        assert!(is_target_date("2027-01-03", &cfg));
        assert!(!is_target_date("2027-01-04", &cfg));
    }

    #[test]
    fn desired_time_filtering() {
        let cfg = sample_config();
        assert!(is_desired_time("2026-12-29T09:00:00+01:00", &cfg));
        assert!(is_desired_time("2026-12-29T09:45:00+01:00", &cfg));
        assert!(!is_desired_time("2026-12-29T10:00:00+01:00", &cfg));
        assert!(!is_desired_time("2026-12-29T14:45:00+01:00", &cfg));
        assert!(is_desired_time("2026-12-29T15:00:00+01:00", &cfg));
        assert!(is_desired_time("2026-12-29T16:15:00+01:00", &cfg));
        assert!(!is_desired_time("invalid-timestamp", &cfg));
    }

    #[test]
    fn slot_eligibility_checks_capacity_time_and_seen() {
        let cfg = sample_config();
        let mut notified = BTreeSet::new();
        notified.insert("2026-12-29T09:15:00+01:00".to_string());

        // Capacity must be >= min_tickets (3)
        assert!(!is_eligible_slot(
            "2026-12-29T09:30:00+01:00",
            2,
            &cfg,
            &notified
        ));
        assert!(is_eligible_slot(
            "2026-12-29T09:30:00+01:00",
            3,
            &cfg,
            &notified
        ));

        // Already notified
        assert!(!is_eligible_slot(
            "2026-12-29T09:15:00+01:00",
            5,
            &cfg,
            &notified
        ));

        // Outside desired hours
        assert!(!is_eligible_slot(
            "2026-12-29T11:00:00+01:00",
            5,
            &cfg,
            &notified
        ));
    }
}
