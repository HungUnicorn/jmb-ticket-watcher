use crate::client::WatcherHttpClient;
use crate::config::{AppConfig, SecretsConfig};
use crate::models::{AvailableSlot, CalendarResponse, QuotasResponse};
use anyhow::{Context, Result};
use tracing::debug;

#[allow(async_fn_in_trait)]
pub trait TicketAvailabilityProvider {
    async fn fetch_bookable_calendar_for_month(
        &self,
        cfg: &AppConfig,
        month_start_date: &str,
    ) -> Result<CalendarResponse>;

    async fn fetch_slot_quotas_for_date(
        &self,
        cfg: &AppConfig,
        valid_date: &str,
    ) -> Result<QuotasResponse>;
}

#[allow(async_fn_in_trait)]
pub trait AlertNotifier {
    async fn dispatch_available_slots_alert(
        &self,
        cfg: &AppConfig,
        secrets: &SecretsConfig,
        available_slots: &[AvailableSlot],
    ) -> Result<()>;
}

impl TicketAvailabilityProvider for WatcherHttpClient {
    async fn fetch_bookable_calendar_for_month(
        &self,
        cfg: &AppConfig,
        month_start_date: &str,
    ) -> Result<CalendarResponse> {
        let response = self
            .inner_client()
            .get(&cfg.calendar_url)
            .header("X-Shop-Url", &cfg.shop_domain)
            .query(&[
                ("by_bookable", "true"),
                ("by_ticket_types[]", cfg.ticket_type.as_str()),
                ("by_museum_ids[]", cfg.museum_id.as_str()),
                ("start_at", month_start_date),
            ])
            .send()
            .await
            .with_context(|| format!("calendar request failed for start_at={month_start_date}"))?;

        let status = response.status();
        debug!(url = %cfg.calendar_url, month_start_date, %status, "calendar response received");

        let raw_body = response
            .error_for_status()
            .with_context(|| {
                format!("calendar endpoint returned error status for {month_start_date}")
            })?
            .text()
            .await
            .with_context(|| format!("failed to read calendar body for {month_start_date}"))?;

        parse_calendar_response_json(&raw_body, month_start_date)
    }

    async fn fetch_slot_quotas_for_date(
        &self,
        cfg: &AppConfig,
        valid_date: &str,
    ) -> Result<QuotasResponse> {
        let response = self
            .inner_client()
            .get(&cfg.quotas_url)
            .header("X-Shop-Url", &cfg.shop_domain)
            .query(&[
                ("by_bookable", "true"),
                ("by_ticket_types[]", cfg.ticket_type.as_str()),
                ("by_museum_ids[]", cfg.museum_id.as_str()),
                ("valid_at", valid_date),
            ])
            .send()
            .await
            .with_context(|| format!("quotas request failed for valid_at={valid_date}"))?;

        let status = response.status();
        debug!(url = %cfg.quotas_url, valid_date, %status, "quotas response received");

        let raw_body = response
            .error_for_status()
            .with_context(|| format!("quotas endpoint returned error status for {valid_date}"))?
            .text()
            .await
            .with_context(|| format!("failed to read quotas body for {valid_date}"))?;

        parse_quotas_response_json(&raw_body, valid_date)
    }
}

impl AlertNotifier for WatcherHttpClient {
    async fn dispatch_available_slots_alert(
        &self,
        cfg: &AppConfig,
        secrets: &SecretsConfig,
        available_slots: &[AvailableSlot],
    ) -> Result<()> {
        let endpoint_url = build_ntfy_endpoint_url(secrets);
        let base_url = secrets.ntfy_base_url.trim_end_matches('/');
        let notification_body =
            format_available_slots_notification_body(available_slots, &cfg.booking_page_url);

        let response = self
            .inner_client()
            .post(&endpoint_url)
            .header("Title", "JMB Time Slot Alert!")
            .header("Priority", "high")
            .header("Tags", "ticket")
            .header("Click", &cfg.booking_page_url)
            .body(notification_body)
            .send()
            .await
            .with_context(|| format!("failed to send ntfy alert to {base_url}/<redacted>"))?;

        let status = response.status();
        debug!(%status, "ntfy response received");

        response
            .error_for_status()
            .context("ntfy endpoint returned error status")?;

        Ok(())
    }
}

pub fn parse_calendar_response_json(
    raw_json: &str,
    month_start_date: &str,
) -> Result<CalendarResponse> {
    serde_json::from_str::<CalendarResponse>(raw_json)
        .with_context(|| format!("failed to decode calendar JSON for {month_start_date}"))
}

pub fn parse_quotas_response_json(raw_json: &str, valid_date: &str) -> Result<QuotasResponse> {
    serde_json::from_str::<QuotasResponse>(raw_json)
        .with_context(|| format!("failed to decode quotas JSON for {valid_date}"))
}

pub fn build_ntfy_endpoint_url(secrets: &SecretsConfig) -> String {
    let base_url = secrets.ntfy_base_url.trim_end_matches('/');
    let topic = secrets.ntfy_topic.trim();
    format!("{base_url}/{topic}")
}

pub fn format_available_slots_notification_body(
    slots: &[AvailableSlot],
    booking_page_url: &str,
) -> String {
    let mut lines = Vec::with_capacity(slots.len() + 3);
    lines.push("New availability found:".to_string());
    for slot in slots {
        let display_timestamp = slot
            .timestamp
            .split_once('+')
            .map(|(prefix, _)| prefix)
            .unwrap_or(&slot.timestamp)
            .replace('T', " ");
        let date_and_minute = display_timestamp.get(..16).unwrap_or(&display_timestamp);
        lines.push(format!("- {date_and_minute} ({} tickets)", slot.capacity));
    }
    lines.push(format!("Book: {booking_page_url}"));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_notification_body_with_slots_and_booking_link() {
        let slots = vec![
            AvailableSlot {
                timestamp: "2026-12-29T09:30:00+01:00".to_string(),
                capacity: 3,
            },
            AvailableSlot {
                timestamp: "2026-12-29T15:00:00+01:00".to_string(),
                capacity: 5,
            },
        ];

        let body = format_available_slots_notification_body(
            &slots,
            "https://tickets.jmberlin.de/tickets/?museum_id=2",
        );

        assert_eq!(
            body,
            "New availability found:\n- 2026-12-29 09:30 (3 tickets)\n- 2026-12-29 15:00 (5 tickets)\nBook: https://tickets.jmberlin.de/tickets/?museum_id=2"
        );
    }

    #[test]
    fn parses_calendar_response_json_into_bookable_map() {
        let raw_json = r#"{"data":{"2026-12-25":true,"2026-12-26":false}}"#;
        let calendar = parse_calendar_response_json(raw_json, "2026-12-01").unwrap();

        assert_eq!(calendar.data.get("2026-12-25"), Some(&true));
        assert_eq!(calendar.data.get("2026-12-26"), Some(&false));
    }

    #[test]
    fn parses_quotas_response_json_into_slot_capacities() {
        let raw_json = r#"{"quotas":{"24":{"capacities":{"2026-12-29T10:00:00+01:00":2}}}}"#;
        let quotas = parse_quotas_response_json(raw_json, "2026-12-29").unwrap();

        assert_eq!(
            quotas.quotas["24"]
                .capacities
                .get("2026-12-29T10:00:00+01:00"),
            Some(&2)
        );
    }

    #[test]
    fn builds_normalized_ntfy_endpoint_url() {
        let secrets = SecretsConfig {
            ntfy_base_url: "https://ntfy.sh/".to_string(),
            ntfy_topic: " my-topic ".to_string(),
        };

        assert_eq!(
            build_ntfy_endpoint_url(&secrets),
            "https://ntfy.sh/my-topic"
        );
    }
}
