use crate::models::{CheckSnapshot, StateFile};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Loads the state from `path`, or creates an empty state file if it does not exist.
pub fn load_or_init_state(path: &Path) -> Result<StateFile> {
    if !path.exists() {
        let empty = StateFile::default();
        save_state(path, &empty)?;
        return Ok(empty);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read state file {}", path.display()))?;
    let parsed: StateFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse state file {}", path.display()))?;
    Ok(parsed)
}

/// Appends a check snapshot and trims `recent_checks` to the last `max_recent_checks` runs.
pub fn record_check_snapshot(
    state: &mut StateFile,
    snapshot: CheckSnapshot,
    max_recent_checks: usize,
) {
    state.recent_checks.push(snapshot);
    if max_recent_checks > 0 && state.recent_checks.len() > max_recent_checks {
        let excess = state.recent_checks.len() - max_recent_checks;
        state.recent_checks.drain(0..excess);
    }
}

/// Persists `StateFile` as formatted JSON to `path`.
pub fn save_state(path: &Path, state: &StateFile) -> Result<()> {
    let serialized =
        serde_json::to_string_pretty(state).context("failed to serialize state JSON")?;

    fs::write(path, format!("{serialized}\n"))
        .with_context(|| format!("failed to write state file {}", path.display()))?;
    Ok(())
}

/// Formats the current UTC time as an ISO-8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn now_utc_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Howard Hinnant's civil_from_days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn keeps_only_last_three_checks() {
        let mut state = StateFile::default();
        for i in 1..=5 {
            record_check_snapshot(
                &mut state,
                CheckSnapshot {
                    checked_at: format!("2026-11-17T20:0{i}:00Z"),
                    bookable_dates: vec!["2026-12-29".to_string()],
                    available_slots: BTreeMap::new(),
                },
                3,
            );
        }
        assert_eq!(state.recent_checks.len(), 3);
        assert_eq!(state.recent_checks[0].checked_at, "2026-11-17T20:03:00Z");
        assert_eq!(state.recent_checks[1].checked_at, "2026-11-17T20:04:00Z");
        assert_eq!(state.recent_checks[2].checked_at, "2026-11-17T20:05:00Z");
    }
}
