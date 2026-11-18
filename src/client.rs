use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

const USER_AGENT: &str = "jmb-ticket-watcher/0.1.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct WatcherHttpClient {
    http_client: Client,
}

impl WatcherHttpClient {
    pub fn new_with_default_timeouts() -> Result<Self> {
        let http_client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { http_client })
    }

    pub fn inner_client(&self) -> &Client {
        &self.http_client
    }
}
