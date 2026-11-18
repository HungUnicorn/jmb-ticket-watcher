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

        response
            .error_for_status()
            .with_context(|| {
                format!("calendar endpoint returned error status for {month_start_date}")
            })?
            .json::<CalendarResponse>()
            .await
            .with_context(|| format!("failed to decode calendar JSON for {month_start_date}"))
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

        response
            .error_for_status()
            .with_context(|| format!("quotas endpoint returned error status for {valid_date}"))?
            .json::<QuotasResponse>()
            .await
            .with_context(|| format!("failed to decode quotas JSON for {valid_date}"))
    }
}

impl AlertNotifier for WatcherHttpClient {
    async fn dispatch_available_slots_alert(
        &self,
        cfg: &AppConfig,
        secrets: &SecretsConfig,
        available_slots: &[AvailableSlot],
    ) -> Result<()> {
        let base_url = secrets.ntfy_base_url.trim_end_matches('/');
        let topic = secrets.ntfy_topic.trim();
        let endpoint_url = format!("{base_url}/{topic}");
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
    use crate::config::{AppConfig, SecretsConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_single_response_http_server(
        status_line: &str,
        content_type: &str,
        body: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let status_line = status_line.to_string();
        let content_type = content_type.to_string();
        let body = body.to_string();

        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let bytes_read = socket.read(&mut buffer).await.unwrap();
            let raw_request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();

            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            raw_request
        });

        (format!("http://{addr}"), handle)
    }

    fn test_app_config(base_url: &str) -> AppConfig {
        AppConfig {
            museum_id: "2".to_string(),
            ticket_type: "time_slot".to_string(),
            shop_domain: "tickets.jmberlin.de".to_string(),
            calendar_url: format!("{base_url}/api/v4/tickets/calendar"),
            quotas_url: format!("{base_url}/api/v4/tickets/list_and_quotas"),
            booking_page_url: "https://tickets.jmberlin.de/tickets/?museum_id=2".to_string(),
            calendar_months: vec!["2026-12-01".to_string()],
            date_start: "2026-12-24".to_string(),
            date_end: "2027-01-03".to_string(),
            excluded_dates: vec!["2026-12-24".to_string()],
            min_tickets: 2,
            morning_before_hour: 10,
            afternoon_from_hour: 15,
            base_interval_secs: 300,
            max_jitter_secs: 300,
            max_recent_checks: 3,
            state_file_path: "last_seen_slots.json".to_string(),
        }
    }

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

    #[tokio::test]
    async fn fetches_calendar_with_shop_header_and_query_params() {
        let (base_url, server_handle) = spawn_single_response_http_server(
            "200 OK",
            "application/json",
            r#"{"data":{"2026-12-25":true,"2026-12-26":false}}"#,
        )
        .await;

        let client = WatcherHttpClient::new_with_default_timeouts().unwrap();
        let cfg = test_app_config(&base_url);

        let calendar = client
            .fetch_bookable_calendar_for_month(&cfg, "2026-12-01")
            .await
            .unwrap();
        let raw_request = server_handle.await.unwrap().to_lowercase();

        assert_eq!(calendar.data.get("2026-12-25"), Some(&true));
        assert_eq!(calendar.data.get("2026-12-26"), Some(&false));
        assert!(raw_request.contains("x-shop-url: tickets.jmberlin.de"));
        assert!(raw_request.contains("start_at=2026-12-01"));
    }

    #[tokio::test]
    async fn fetches_quotas_for_date_and_parses_capacities() {
        let (base_url, server_handle) = spawn_single_response_http_server(
            "200 OK",
            "application/json",
            r#"{"quotas":{"24":{"capacities":{"2026-12-29T10:00:00+01:00":2}}}}"#,
        )
        .await;

        let client = WatcherHttpClient::new_with_default_timeouts().unwrap();
        let cfg = test_app_config(&base_url);

        let quotas = client
            .fetch_slot_quotas_for_date(&cfg, "2026-12-29")
            .await
            .unwrap();
        let raw_request = server_handle.await.unwrap().to_lowercase();

        assert_eq!(
            quotas.quotas["24"]
                .capacities
                .get("2026-12-29T10:00:00+01:00"),
            Some(&2)
        );
        assert!(raw_request.contains("x-shop-url: tickets.jmberlin.de"));
        assert!(raw_request.contains("valid_at=2026-12-29"));
    }

    #[tokio::test]
    async fn dispatches_ntfy_alert_with_expected_headers_and_body() {
        let (base_url, server_handle) =
            spawn_single_response_http_server("200 OK", "application/json", r#"{"id":"ok"}"#).await;

        let client = WatcherHttpClient::new_with_default_timeouts().unwrap();
        let cfg = test_app_config(&base_url);
        let secrets = SecretsConfig {
            ntfy_base_url: base_url,
            ntfy_topic: "test-topic".to_string(),
        };
        let slots = vec![AvailableSlot {
            timestamp: "2026-12-29T10:00:00+01:00".to_string(),
            capacity: 2,
        }];

        client
            .dispatch_available_slots_alert(&cfg, &secrets, &slots)
            .await
            .unwrap();
        let raw_request = server_handle.await.unwrap();
        let lower = raw_request.to_lowercase();

        assert!(raw_request.starts_with("POST /test-topic "));
        assert!(lower.contains("title: jmb time slot alert!"));
        assert!(lower.contains("priority: high"));
        assert!(raw_request.contains("- 2026-12-29 10:00 (2 tickets)"));
    }
}
