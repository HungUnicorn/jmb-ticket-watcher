use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

const USER_AGENT: &str = "jmb-ticket-watcher/0.1.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the shared HTTP client with default timeouts and User-Agent.
pub fn build_http_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .context("failed to build HTTP client")
}

