use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Deserialize)]
pub struct CalendarResponse {
    pub data: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
pub struct QuotasResponse {
    #[serde(default)]
    pub quotas: HashMap<String, QuotaEntry>,
}

#[derive(Debug, Deserialize)]
pub struct QuotaEntry {
    #[serde(default)]
    pub capacities: HashMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckSnapshot {
    pub checked_at: String,
    pub bookable_dates: Vec<String>,
    pub available_slots: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct StateFile {
    #[serde(default, alias = "seen_slots")]
    pub notified_slots: BTreeSet<String>,
    #[serde(default)]
    pub recent_checks: Vec<CheckSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AvailableSlot {
    pub timestamp: String,
    pub capacity: i64,
}
