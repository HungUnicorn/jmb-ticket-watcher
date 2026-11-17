mod api;
mod client;
mod config;
mod models;
mod state;

use crate::api::{
    fetch_calendar, fetch_quotas, format_alert_message, is_eligible_slot, is_target_date,
    send_ntfy_alert,
};
use crate::client::build_http_client;
use crate::config::{
    load_config, load_secrets, AppConfig, SecretsConfig, CONFIG_FILE_PATH, SECRETS_FILE_PATH,
};
use crate::models::{AvailableSlot, CheckSnapshot, StateFile};
use crate::state::{load_or_init_state, now_utc_iso8601, record_check_snapshot, save_state};
use anyhow::Result;
use rand::Rng;
use reqwest::Client;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let run_once = args.iter().any(|a| a == "--once");

    let config_path = Path::new(CONFIG_FILE_PATH);
    let secrets_path = Path::new(SECRETS_FILE_PATH);

    let mut cfg = load_config(config_path)?;
    let mut secrets = load_secrets(secrets_path)?;

    let client = build_http_client()?;
    let state_path = Path::new(&cfg.state_file_path).to_path_buf();
    let mut state = load_or_init_state(&state_path)?;

    info!(
        config_file = CONFIG_FILE_PATH,
        secrets_file = SECRETS_FILE_PATH,
        state_file = %state_path.display(),
        min_tickets = cfg.min_tickets,
        morning_before_hour = cfg.morning_before_hour,
        afternoon_from_hour = cfg.afternoon_from_hour,
        notified_slots = state.notified_slots.len(),
        recent_checks = state.recent_checks.len(),
        "Starting jmb-ticket-watcher"
    );

    loop {
        // Reload config.json and secrets.json on each cycle so parameter edits apply without restarting.
        if let Ok(updated_cfg) = load_config(config_path) {
            cfg = updated_cfg;
        }
        if let Ok(updated_secrets) = load_secrets(secrets_path) {
            secrets = updated_secrets;
        }

        if let Err(err) = run_cycle(&client, &cfg, &secrets, &state_path, &mut state).await {
            error!(error = %err, "Polling cycle failed");
        }

        if run_once {
            break;
        }

        let sleep_secs =
            cfg.base_interval_secs + rand::thread_rng().gen_range(0..=cfg.max_jitter_secs);
        info!("Sleeping for {sleep_secs}s until next cycle.");

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down.");
                break;
            }
        }
    }

    Ok(())
}

async fn run_cycle(
    client: &Client,
    cfg: &AppConfig,
    secrets: &SecretsConfig,
    state_path: &Path,
    state: &mut StateFile,
) -> Result<()> {
    info!(
        min_tickets = cfg.min_tickets,
        morning_before_hour = cfg.morning_before_hour,
        afternoon_from_hour = cfg.afternoon_from_hour,
        "Checking JMB availability..."
    );

    // Step 1: Calendar sweep across target months.
    let mut bookable_dates: Vec<String> = Vec::new();
    for month in &cfg.calendar_months {
        let calendar = fetch_calendar(client, cfg, month).await?;
        for (date, bookable) in calendar.data {
            if bookable && is_target_date(&date, cfg) {
                bookable_dates.push(date);
            }
        }
    }
    bookable_dates.sort();
    bookable_dates.dedup();

    let mut available_slots: BTreeMap<String, i64> = BTreeMap::new();
    let mut new_slots: Vec<AvailableSlot> = Vec::new();

    if bookable_dates.is_empty() {
        info!("No bookable dates found in target window.");
    } else {
        info!(?bookable_dates, "Found bookable dates in target window");

        // Step 2: Quota & capacity verification for each bookable date.
        for date in &bookable_dates {
            match fetch_quotas(client, cfg, date).await {
                Ok(quotas_resp) => {
                    for quota in quotas_resp.quotas.values() {
                        for (timestamp, &capacity) in &quota.capacities {
                            if capacity > 0 {
                                available_slots.insert(timestamp.clone(), capacity);
                            }
                            if is_eligible_slot(timestamp, capacity, cfg, &state.notified_slots) {
                                new_slots.push(AvailableSlot {
                                    timestamp: timestamp.clone(),
                                    capacity,
                                });
                            }
                        }
                    }
                }
                Err(err) => {
                    warn!(date, error = %err, "Failed to fetch quotas for date");
                }
            }
        }
    }

    // Record this check in the rolling check history and persist immediately.
    record_check_snapshot(
        state,
        CheckSnapshot {
            checked_at: now_utc_iso8601(),
            bookable_dates,
            available_slots,
        },
        cfg.max_recent_checks,
    );
    save_state(state_path, state)?;

    new_slots.sort();

    // Step 3: Alert & deduplication state update.
    if new_slots.is_empty() {
        info!(
            state_file = %state_path.display(),
            "No new eligible slots found. Saved check snapshot."
        );
        return Ok(());
    }

    let alert_text = format_alert_message(&new_slots, &cfg.booking_page_url);
    info!("\n{alert_text}");

    send_ntfy_alert(client, cfg, secrets, &new_slots).await?;

    for slot in &new_slots {
        state.notified_slots.insert(slot.timestamp.clone());
    }
    save_state(state_path, state)?;
    info!(
        state_file = %state_path.display(),
        "Dispatched ntfy alert and recorded {} new slot(s).",
        new_slots.len()
    );

    Ok(())
}
