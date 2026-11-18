mod api;
mod client;
mod config;
mod models;
mod state;
mod watcher;

use crate::client::WatcherHttpClient;
use crate::config::{
    ConfigurationSource, JsonFileConfigurationSource, CONFIG_FILE_PATH, FILTERS_FILE_PATH,
    SECRETS_FILE_PATH,
};
use crate::state::{JsonFileStateRepository, StateRepository};
use crate::watcher::{compute_randomized_sleep_seconds, PollingTicketWatcher, TicketWatcher};
use anyhow::Result;
use std::time::Duration;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let run_single_cycle_only = std::env::args().any(|arg| arg == "--once");

    let config_source =
        JsonFileConfigurationSource::new(CONFIG_FILE_PATH, FILTERS_FILE_PATH, SECRETS_FILE_PATH);
    let mut cfg = config_source.load_app_config()?;
    let mut secrets = config_source.load_secrets_config()?;

    let http_client = WatcherHttpClient::new_with_default_timeouts()?;
    let state_repository = JsonFileStateRepository::new(&cfg.state_file_path);
    let mut state = state_repository.load_or_initialize_state()?;
    let watcher = PollingTicketWatcher::new(&http_client, &http_client, &state_repository);

    info!(
        config_file = CONFIG_FILE_PATH,
        filters_file = FILTERS_FILE_PATH,
        secrets_file = SECRETS_FILE_PATH,
        state_file = %state_repository.file_path().display(),
        min_available_tickets = cfg.min_available_tickets,
        morning_before_hour = cfg.morning_before_hour,
        afternoon_from_hour = cfg.afternoon_from_hour,
        notified_slots = state.notified_slots.len(),
        recent_checks = state.recent_checks.len(),
        "Starting jmb-ticket-watcher"
    );

    loop {
        if let Ok(reloaded_cfg) = config_source.load_app_config() {
            cfg = reloaded_cfg;
        }
        if let Ok(reloaded_secrets) = config_source.load_secrets_config() {
            secrets = reloaded_secrets;
        }

        if let Err(err) = watcher
            .execute_polling_cycle(&cfg, &secrets, &mut state)
            .await
        {
            error!(error = %err, "Polling cycle failed");
        }

        if run_single_cycle_only {
            break;
        }

        let sleep_seconds = compute_randomized_sleep_seconds(&cfg);
        info!("Sleeping for {sleep_seconds}s until next cycle.");

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(sleep_seconds)) => {}
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down.");
                break;
            }
        }
    }

    Ok(())
}
