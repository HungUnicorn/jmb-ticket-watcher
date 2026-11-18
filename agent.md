# agent.md

## 1. Overview & Objective

Build an automated, headless daemon in Rust (`jmb-ticket-watcher`) running on macOS (Apple Silicon) under `caffeinate -i`. The binary monitors time-slot availability on a JMB/gomus calendar (configured via `config.json`) for a target date window (default **2026-12-24 to 2027-01-03**, excluding **2026-12-24**).

Eligible time slots must have at least `min_tickets` available (default `2`) and fall within the desired time windows: up to `10:00` inclusive (`<= 10:00`) or from `15:00` onwards (`>= 15:00`).

The daemon deduplicates notified slots and records a rolling history of the last 3 polling checks in `last_seen_slots.json`, and sends push alerts via **ntfy** (`ntfy.sh`) with a direct booking link.

---

## 2. API Specifications & Endpoints

All requests to `jmb.gomus.de` **must** include the header:
* `X-Shop-Url: tickets.jmberlin.de`

### 2.1 Calendar Availability Check

* **Base URL:** `https://jmb.gomus.de/api/v4/tickets/calendar`
* **Headers:**
  * `X-Shop-Url: tickets.jmberlin.de`
* **Query Parameters:**
  * `by_bookable=true`
  * `by_ticket_types[]=time_slot`
  * `by_museum_ids[]=2`
  * `start_at={YYYY-MM-01}` (Queried for each entry in `calendar_months`, e.g., `2026-12-01` and `2027-01-01`)
* **Expected Response Schema:**
```json
{
  "data": {
    "2026-12-24": false,
    "2026-12-25": true,
    "2026-12-26": false
  }
}
```

### 2.2 Quota & Capacity Verification

* **Base URL:** `https://jmb.gomus.de/api/v4/tickets/list_and_quotas`
* **Headers:**
  * `X-Shop-Url: tickets.jmberlin.de`
* **Query Parameters:**
  * `by_bookable=true`
  * `by_ticket_types[]=time_slot`
  * `by_museum_ids[]=2`
  * `valid_at={YYYY-MM-DD}` (Only invoked for bookable dates in the target window)
* **Expected Response Schema:**
```json
{
  "quotas": {
    "24": {
      "capacities": {
        "2026-12-29T10:00:00+01:00": 1,
        "2026-12-29T10:15:00+01:00": 0
      }
    }
  }
}
```

### 2.3 ntfy Push Alert Dispatch

* **Endpoint:** `<ntfy_base_url>/<ntfy_topic>` (default base URL: `https://ntfy.sh`)
* **Method:** `POST`
* **Headers:**
  * `Title: JMB Time Slot Alert!`
  * `Priority: high`
  * `Tags: ticket`
  * `Click: https://tickets.jmberlin.de/tickets/?museum_id=2`
* **Body (plain text):**
```text
New availability found:
- 2026-12-29 09:30 (3 tickets)
Book: https://tickets.jmberlin.de/tickets/?museum_id=2
```

---

## 3. Configuration & State Management

Configuration is split between non-sensitive parameters (`config.json`) and sensitive credentials (`secrets.json`), both reloaded on each polling cycle.

### 3.1 `config.json`

* `museum_id`: `"2"`
* `ticket_type`: `"time_slot"`
* `shop_domain`: `"tickets.jmberlin.de"`
* `calendar_url` / `quotas_url` / `booking_page_url`
* `calendar_months`: `["2026-12-01", "2027-01-01"]`
* `date_start` / `date_end`: `"2026-12-24"` through `"2027-01-03"` inclusive
* `excluded_dates`: `["2026-12-24"]`
* `min_tickets`: `2` (`capacity >= min_tickets`)
* `morning_before_hour`: `10` (`<= 10:00`)
* `afternoon_from_hour`: `15` (`>= 15:00`)
* `base_interval_secs`: `300` (5 minutes)
* `max_jitter_secs`: `300` (0–300s random jitter, 5–10 minute window)
* `max_recent_checks`: `3`
* `state_file_path`: `"last_seen_slots.json"`

### 3.2 `secrets.json` (gitignored)

* `ntfy_base_url`: `"https://ntfy.sh"` (optional, defaults to `https://ntfy.sh`)
* `ntfy_topic`: Target ntfy topic string (can also be overridden via `NTFY_TOPIC` environment variable)

See `secrets.example.json` for the template.

### 3.3 State File (`last_seen_slots.json`, gitignored)

* Persisted in the working directory.
* Schema:
```json
{
  "notified_slots": [
    "2026-12-29T09:30:00+01:00"
  ],
  "recent_checks": [
    {
      "checked_at": "2026-11-17T19:16:32Z",
      "bookable_dates": [
        "2026-12-25",
        "2026-12-27",
        "2026-12-29"
      ],
      "available_slots": {
        "2026-12-25T11:45:00+01:00": 1,
        "2026-12-29T10:00:00+01:00": 1
      }
    }
  ]
}
```
* `notified_slots`: Deduplication set of timestamps that have already triggered an ntfy alert.
* `recent_checks`: Rolling snapshot of the last `max_recent_checks` (3) polling cycles, recording all slots with `capacity > 0` for easy verification.

---

## 4. Architecture & Component Design

```
├── config.json             # Non-sensitive runtime & filter configuration
├── secrets.json            # Sensitive ntfy topic configuration (gitignored)
├── secrets.example.json    # Template for secrets.json
├── last_seen_slots.json    # Persisted state & last 3 check snapshots (gitignored)
├── Cargo.toml
└── src/
    ├── main.rs             # Entrypoint, --once flag, polling loop, Ctrl+C handling, jitter
    ├── config.rs           # AppConfig & SecretsConfig loaders
    ├── client.rs           # HTTP client setup (custom User-Agent, timeout defaults)
    ├── api.rs              # JMB/gomus queries, slot filtering, ntfy alert dispatch
    ├── models.rs           # Serde structs for calendar, quotas, snapshots, and state JSON
    └── state.rs            # Load/save and rolling 3-check history management
```

### 4.1 Dependency Manifest (`Cargo.toml`)

```toml
[package]
name = "jmb-ticket-watcher"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
rand = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

## 5. Execution Pipeline (Per Polling Cycle)

1. **Config Reload & Cycle Start:**
   * Reload `config.json` and `secrets.json` (with optional `NTFY_TOPIC` env override).
   * Log `Checking JMB availability...` with active filter thresholds.

2. **Step 1: Calendar Sweep:**
   * Fetch `/api/v4/tickets/calendar` for each month in `calendar_months` with `X-Shop-Url`.
   * Retain dates where `bookable == true`, `date >= date_start && date <= date_end`, and `!excluded_dates.contains(date)`.

3. **Step 2: Quota Extraction:**
   * For each bookable date, fetch `/api/v4/tickets/list_and_quotas?valid_at={date}` with `X-Shop-Url`.
   * Collect all slots with `capacity > 0` into `available_slots` for the check snapshot.
   * Collect eligible slots where `capacity >= min_tickets`, `hour < morning_before_hour || hour >= afternoon_from_hour`, and `!notified_slots.contains(timestamp)`.

4. **Step 3: Snapshot, Alert & State Update:**
   * Append the cycle's `CheckSnapshot` to `recent_checks` (keeping the last `max_recent_checks` entries) and persist `last_seen_slots.json`.
   * If `new_slots` is non-empty:
     * Send push alert to `ntfy` (`POST <ntfy_base_url>/<ntfy_topic>`).
     * On success, insert all `new_slots` timestamps into `notified_slots` and persist `last_seen_slots.json`.

5. **Step 4: Sleep Interval (unless `--once` is passed):**
   * Sleep `base_interval_secs + rand(0..=max_jitter_secs)` seconds, waking early on `Ctrl+C`.

---

## 6. Build & Operational Runbook

### 6.1 Compilation & Tests

```bash
cargo test
cargo build --release
```

### 6.2 Single-Run Verification

```bash
./target/release/jmb-ticket-watcher --once
```

### 6.3 Run with Sleep Prevention (`caffeinate`)

```bash
caffeinate -i ./target/release/jmb-ticket-watcher
```

### 6.4 Stopping the Daemon

```bash
pkill -f jmb-ticket-watcher
```
