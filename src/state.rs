use crate::models::{AvailableSlot, CheckSnapshot, StateFile};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait StateRepository {
    fn load_or_initialize_state(&self) -> Result<StateFile>;
    fn persist_state(&self, state: &StateFile) -> Result<()>;
}

pub struct JsonFileStateRepository {
    file_path: PathBuf,
}

impl JsonFileStateRepository {
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        Self {
            file_path: file_path.into(),
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }
}

impl StateRepository for JsonFileStateRepository {
    fn load_or_initialize_state(&self) -> Result<StateFile> {
        if !self.file_path.exists() {
            let empty_state = StateFile::default();
            self.persist_state(&empty_state)?;
            return Ok(empty_state);
        }

        let raw = fs::read_to_string(&self.file_path)
            .with_context(|| format!("failed to read state file {}", self.file_path.display()))?;
        let parsed: StateFile = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse state file {}", self.file_path.display()))?;
        Ok(parsed)
    }

    fn persist_state(&self, state: &StateFile) -> Result<()> {
        let serialized =
            serde_json::to_string_pretty(state).context("failed to serialize state JSON")?;

        fs::write(&self.file_path, format!("{serialized}\n"))
            .with_context(|| format!("failed to write state file {}", self.file_path.display()))?;
        Ok(())
    }
}

pub trait SlotStateTracker {
    fn append_check_snapshot_within_limit(
        &mut self,
        snapshot: CheckSnapshot,
        max_recent_checks: usize,
    );
    fn mark_slots_as_notified(&mut self, notified_slots: &[AvailableSlot]);
}

impl SlotStateTracker for StateFile {
    fn append_check_snapshot_within_limit(
        &mut self,
        snapshot: CheckSnapshot,
        max_recent_checks: usize,
    ) {
        self.recent_checks.push(snapshot);
        if max_recent_checks > 0 && self.recent_checks.len() > max_recent_checks {
            let excess = self.recent_checks.len() - max_recent_checks;
            self.recent_checks.drain(0..excess);
        }
    }

    fn mark_slots_as_notified(&mut self, notified_slots: &[AvailableSlot]) {
        for slot in notified_slots {
            self.notified_slots.insert(slot.timestamp.clone());
        }
    }
}

pub fn format_current_utc_timestamp_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

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
            state.append_check_snapshot_within_limit(
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
