# jmb-ticket-watcher

A lightweight Rust daemon that monitors time-slot ticket availability for the **Jewish Museum Berlin** (`museum_id=2`) and sends instant push notifications via [ntfy](https://ntfy.sh) when matching slots open up.

## Features

- Polls the JMB/gomus calendar and quota APIs on a randomized 5–10 minute interval (`300s` base + `0–300s` jitter).
- Filters slots by date range (`2026-12-24` to `2027-01-03`, excluding `2026-12-24`), minimum available tickets (`min_tickets`), and preferred hours (up to `10:00` inclusive or from `15:00` onwards).
- Hot-reloads `config.json` and `secrets.json` on every polling cycle — no restart needed when tuning filters.
- Deduplicates alerts in `last_seen_slots.json` (`notified_slots`) and records a rolling history of the last 3 checks (`recent_checks`) for easy verification.
- Supports `--once` for a single check and exits cleanly on `Ctrl+C`.

## Setup

1. **Create `secrets.json`** from the example template:
   ```bash
   cp secrets.example.json secrets.json
   ```
   Edit `secrets.json` and set a private, hard-to-guess `ntfy_topic`:
   ```json
   {
     "ntfy_base_url": "https://ntfy.sh",
     "ntfy_topic": "your-secret-ntfy-topic"
   }
   ```
   Subscribe to the same topic in the **ntfy** app on your phone (iOS/Android).

2. **Review `config.json`** to adjust your date window, `min_tickets`, or time-of-day filters if needed:
   ```json
   {
     "min_tickets": 2,
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

