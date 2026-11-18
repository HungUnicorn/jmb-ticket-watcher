# jmb-ticket-watcher

[![CI](https://github.com/HungUnicorn/jmb-ticket-watcher/actions/workflows/ci.yml/badge.svg)](https://github.com/HungUnicorn/jmb-ticket-watcher/actions/workflows/ci.yml)

A lightweight Rust daemon that monitors time-slot ticket availability on a **JMB/gomus** ticket calendar (configured via `config.json`) and sends instant push notifications via [ntfy](https://ntfy.sh) when matching slots open up.

## Features

- Polls the JMB/gomus calendar and quota APIs on a randomized 5–10 minute interval (`300s` base + `0–300s` jitter).
- Filters slots by date range (`2026-12-24` to `2027-01-03`, excluding `2026-12-24`), minimum available tickets (`min_available_tickets`), and preferred hours (up to `10:00` inclusive or from `15:00` onwards).
- Hot-reloads `config/config.json` (system settings), `config/filters.json` (personal filter criteria), and `config/secrets.json` (ntfy credentials) on every polling cycle — no restart needed when tuning filters.
- Deduplicates alerts in `data/last_seen_slots.json` (`notified_slots`) and records a rolling history of the last 3 checks (`recent_checks`) for easy verification.
- Supports `--once` for a single check and exits cleanly on `Ctrl+C`.

## Setup

1. **Create `config/secrets.json` and `config/filters.json`** from the example templates:
   ```bash
   cp config/secrets.example.json config/secrets.json
   cp config/filters.example.json config/filters.json
   ```
   Edit `config/secrets.json` and set a private, hard-to-guess `ntfy_topic`:
   ```json
   {
     "ntfy_base_url": "https://ntfy.sh",
     "ntfy_topic": "your-secret-ntfy-topic"
   }
   ```
   Subscribe to the same topic in the **ntfy** app on your phone (iOS/Android).

2. **Edit `config/filters.json`** to set your personal target `museum_id`, date window, `min_available_tickets`, and time-of-day filters:
   ```json
   {
     "museum_id": "2",
     "calendar_months": ["2026-12-01", "2027-01-01"],
     "date_start": "2026-12-24",
     "date_end": "2027-01-03",
     "excluded_dates": ["2026-12-24"],
     "min_available_tickets": 2,
     "morning_before_hour": 10,
     "afternoon_from_hour": 15
   }
   ```

3. **Build & Test:**
   ```bash
   cargo test
   cargo build --release
   ```

## Usage

Run a single check:
```bash
./target/release/jmb-ticket-watcher --once
```

Run continuously on macOS while preventing idle sleep:
```bash
caffeinate -i ./target/release/jmb-ticket-watcher
```

Check if it is running:
```bash
pgrep -l jmb-ticket-watcher
```

Stop the background daemon:
```bash
pkill -f jmb-ticket-watcher
```

## License

This project is licensed under the [MIT License](LICENSE).

