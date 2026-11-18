use crate::api::{
    format_available_slots_notification_body, AlertNotifier, TicketAvailabilityProvider,
};
use crate::config::{AppConfig, SecretsConfig, TicketSlotFilter};
use crate::models::{AvailableSlot, CheckSnapshot, StateFile};
use crate::state::{format_current_utc_timestamp_iso8601, SlotStateTracker, StateRepository};
use anyhow::Result;
use rand::Rng;
use std::collections::BTreeMap;
use tracing::{info, warn};

#[allow(async_fn_in_trait)]
pub trait TicketWatcher {
    async fn execute_polling_cycle(
        &self,
        cfg: &AppConfig,
        secrets: &SecretsConfig,
        state: &mut StateFile,
    ) -> Result<()>;
}

pub struct PollingTicketWatcher<'a, P, N, R> {
    availability_provider: &'a P,
    alert_notifier: &'a N,
    state_repository: &'a R,
}

impl<'a, P, N, R> PollingTicketWatcher<'a, P, N, R> {
    pub fn new(
        availability_provider: &'a P,
        alert_notifier: &'a N,
        state_repository: &'a R,
    ) -> Self {
        Self {
            availability_provider,
            alert_notifier,
            state_repository,
        }
    }
}

impl<P, N, R> TicketWatcher for PollingTicketWatcher<'_, P, N, R>
where
    P: TicketAvailabilityProvider,
    N: AlertNotifier,
    R: StateRepository,
{
    async fn execute_polling_cycle(
        &self,
        cfg: &AppConfig,
        secrets: &SecretsConfig,
        state: &mut StateFile,
    ) -> Result<()> {
        execute_polling_cycle(
            self.availability_provider,
            self.alert_notifier,
            self.state_repository,
            cfg,
            secrets,
            state,
        )
        .await
    }
}

pub fn compute_randomized_sleep_seconds(cfg: &AppConfig) -> u64 {
    cfg.base_interval_secs + rand::thread_rng().gen_range(0..=cfg.max_jitter_secs)
}

pub async fn execute_polling_cycle<P, N, R>(
    availability_provider: &P,
    alert_notifier: &N,
    state_repository: &R,
    cfg: &AppConfig,
    secrets: &SecretsConfig,
    state: &mut StateFile,
) -> Result<()>
where
    P: TicketAvailabilityProvider,
    N: AlertNotifier,
    R: StateRepository,
{
    info!(
        min_tickets = cfg.min_tickets,
        morning_before_hour = cfg.morning_before_hour,
        afternoon_from_hour = cfg.afternoon_from_hour,
        "Checking JMB availability..."
    );

    let bookable_dates = collect_bookable_target_dates(availability_provider, cfg).await?;

    let (all_available_slots, mut newly_eligible_slots) = if bookable_dates.is_empty() {
        info!("No bookable dates found in target window.");
        (BTreeMap::new(), Vec::new())
    } else {
        info!(?bookable_dates, "Found bookable dates in target window");
        collect_available_and_eligible_slots_for_dates(
            availability_provider,
            cfg,
            &bookable_dates,
            state,
        )
        .await
    };

    state.append_check_snapshot_within_limit(
        CheckSnapshot {
            checked_at: format_current_utc_timestamp_iso8601(),
            bookable_dates,
            available_slots: all_available_slots,
        },
        cfg.max_recent_checks,
    );
    state_repository.persist_state(state)?;

    newly_eligible_slots.sort();

    if newly_eligible_slots.is_empty() {
        info!("No new eligible slots found. Saved check snapshot.");
        return Ok(());
    }

    let alert_summary =
        format_available_slots_notification_body(&newly_eligible_slots, &cfg.booking_page_url);
    info!("\n{alert_summary}");

    alert_notifier
        .dispatch_available_slots_alert(cfg, secrets, &newly_eligible_slots)
        .await?;

    state.mark_slots_as_notified(&newly_eligible_slots);
    state_repository.persist_state(state)?;
    info!(
        "Dispatched ntfy alert and recorded {} new slot(s).",
        newly_eligible_slots.len()
    );

    Ok(())
}

pub async fn collect_bookable_target_dates<P>(
    availability_provider: &P,
    cfg: &AppConfig,
) -> Result<Vec<String>>
where
    P: TicketAvailabilityProvider,
{
    let mut bookable_dates = Vec::new();
    for month_start in &cfg.calendar_months {
        let calendar = availability_provider
            .fetch_bookable_calendar_for_month(cfg, month_start)
            .await?;
        for (date, is_bookable) in calendar.data {
            if is_bookable && cfg.is_date_within_target_window(&date) {
                bookable_dates.push(date);
            }
        }
    }
    bookable_dates.sort();
    bookable_dates.dedup();
    Ok(bookable_dates)
}

pub async fn collect_available_and_eligible_slots_for_dates<P>(
    availability_provider: &P,
    cfg: &AppConfig,
    bookable_dates: &[String],
    state: &StateFile,
) -> (BTreeMap<String, i64>, Vec<AvailableSlot>)
where
    P: TicketAvailabilityProvider,
{
    let mut all_available_slots = BTreeMap::new();
    let mut newly_eligible_slots = Vec::new();

    for date in bookable_dates {
        match availability_provider
            .fetch_slot_quotas_for_date(cfg, date)
            .await
        {
            Ok(quotas_response) => {
                for quota in quotas_response.quotas.values() {
                    for (timestamp, &capacity) in &quota.capacities {
                        if capacity > 0 {
                            all_available_slots.insert(timestamp.clone(), capacity);
                        }
                        if cfg.is_slot_eligible_for_notification(
                            timestamp,
                            capacity,
                            &state.notified_slots,
                        ) {
                            newly_eligible_slots.push(AvailableSlot {
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

    (all_available_slots, newly_eligible_slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CalendarResponse, QuotaEntry, QuotasResponse};
    use anyhow::anyhow;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct MockAvailabilityProvider {
        calendars: HashMap<String, CalendarResponse>,
        quotas: HashMap<String, QuotasResponse>,
    }

    impl TicketAvailabilityProvider for MockAvailabilityProvider {
        async fn fetch_bookable_calendar_for_month(
            &self,
            _cfg: &AppConfig,
            month_start_date: &str,
        ) -> Result<CalendarResponse> {
            self.calendars
                .get(month_start_date)
                .cloned()
                .ok_or_else(|| anyhow!("missing mock calendar for {month_start_date}"))
        }

        async fn fetch_slot_quotas_for_date(
            &self,
            _cfg: &AppConfig,
            valid_date: &str,
        ) -> Result<QuotasResponse> {
            self.quotas
                .get(valid_date)
                .cloned()
                .ok_or_else(|| anyhow!("missing mock quotas for {valid_date}"))
        }
    }

    struct MockAlertNotifier {
        should_fail: bool,
        dispatched_batches: RefCell<Vec<Vec<AvailableSlot>>>,
    }

    impl MockAlertNotifier {
        fn succeeding() -> Self {
            Self {
                should_fail: false,
                dispatched_batches: RefCell::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                should_fail: true,
                dispatched_batches: RefCell::new(Vec::new()),
            }
        }
    }

    impl AlertNotifier for MockAlertNotifier {
        async fn dispatch_available_slots_alert(
            &self,
            _cfg: &AppConfig,
            _secrets: &SecretsConfig,
            available_slots: &[AvailableSlot],
        ) -> Result<()> {
            if self.should_fail {
                return Err(anyhow!("simulated ntfy failure"));
            }
            self.dispatched_batches
                .borrow_mut()
                .push(available_slots.to_vec());
            Ok(())
        }
    }

    struct InMemoryStateRepository {
        persisted_states: RefCell<Vec<StateFile>>,
    }

    impl InMemoryStateRepository {
        fn new() -> Self {
            Self {
                persisted_states: RefCell::new(Vec::new()),
            }
        }
    }

    impl StateRepository for InMemoryStateRepository {
        fn load_or_initialize_state(&self) -> Result<StateFile> {
            Ok(StateFile::default())
        }

        fn persist_state(&self, state: &StateFile) -> Result<()> {
            self.persisted_states.borrow_mut().push(state.clone());
            Ok(())
        }
    }

    fn test_config() -> AppConfig {
        AppConfig {
            museum_id: "2".to_string(),
            ticket_type: "time_slot".to_string(),
            shop_domain: "tickets.jmberlin.de".to_string(),
            calendar_url: "https://example.test/calendar".to_string(),
            quotas_url: "https://example.test/quotas".to_string(),
            booking_page_url: "https://example.test/book".to_string(),
            calendar_months: vec!["2026-12-01".to_string()],
            date_start: "2026-12-24".to_string(),
            date_end: "2027-01-03".to_string(),
            excluded_dates: vec!["2026-12-24".to_string()],
            min_tickets: 2,
            morning_before_hour: 10,
            afternoon_from_hour: 15,
            base_interval_secs: 300,
            max_jitter_secs: 300,
            max_recent_checks: 3,
            state_file_path: "last_seen_slots.json".to_string(),
        }
    }

    fn test_secrets() -> SecretsConfig {
        SecretsConfig {
            ntfy_base_url: "https://ntfy.sh".to_string(),
            ntfy_topic: "test-topic".to_string(),
        }
    }

    #[tokio::test]
    async fn persists_snapshot_without_alert_when_no_bookable_dates() {
        let mut calendars = HashMap::new();
        calendars.insert(
            "2026-12-01".to_string(),
            CalendarResponse {
                data: HashMap::from([("2026-12-25".to_string(), false)]),
            },
        );
        let provider = MockAvailabilityProvider {
            calendars,
            quotas: HashMap::new(),
        };
        let notifier = MockAlertNotifier::succeeding();
        let repo = InMemoryStateRepository::new();
        let mut state = StateFile::default();

        let watcher = PollingTicketWatcher::new(&provider, &notifier, &repo);

        watcher
            .execute_polling_cycle(&test_config(), &test_secrets(), &mut state)
            .await
            .unwrap();

        assert!(notifier.dispatched_batches.borrow().is_empty());
        assert!(state.notified_slots.is_empty());
        assert_eq!(state.recent_checks.len(), 1);
        assert!(state.recent_checks[0].bookable_dates.is_empty());
        assert_eq!(repo.persisted_states.borrow().len(), 1);
    }

    #[tokio::test]
    async fn dispatches_alert_only_for_eligible_slots_and_deduplicates_next_cycle() {
        let excluded_date = ("2026-12-24".to_string(), true);
        let target_date = ("2026-12-29".to_string(), true);
        let mut calendars = HashMap::new();
        calendars.insert(
            "2026-12-01".to_string(),
            CalendarResponse {
                data: HashMap::from([excluded_date, target_date]),
            },
        );

        let eligible_morning_slot = ("2026-12-29T09:30:00+01:00".to_string(), 3);
        let eligible_morning_cutoff_slot = ("2026-12-29T10:00:00+01:00".to_string(), 2);
        let outside_desired_hours_slot = ("2026-12-29T11:00:00+01:00".to_string(), 5);
        let below_minimum_tickets_slot = ("2026-12-29T15:00:00+01:00".to_string(), 1);
        let capacities = HashMap::from([
            eligible_morning_slot,
            eligible_morning_cutoff_slot,
            outside_desired_hours_slot,
            below_minimum_tickets_slot,
        ]);
        let mut quotas = HashMap::new();
        quotas.insert(
            "2026-12-29".to_string(),
            QuotasResponse {
                quotas: HashMap::from([("24".to_string(), QuotaEntry { capacities })]),
            },
        );

        let provider = MockAvailabilityProvider { calendars, quotas };
        let notifier = MockAlertNotifier::succeeding();
        let repo = InMemoryStateRepository::new();
        let mut state = StateFile::default();

        let watcher = PollingTicketWatcher::new(&provider, &notifier, &repo);

        watcher
            .execute_polling_cycle(&test_config(), &test_secrets(), &mut state)
            .await
            .unwrap();

        assert_eq!(notifier.dispatched_batches.borrow().len(), 1);
        assert_eq!(
            notifier.dispatched_batches.borrow()[0],
            vec![
                AvailableSlot {
                    timestamp: "2026-12-29T09:30:00+01:00".to_string(),
                    capacity: 3,
                },
                AvailableSlot {
                    timestamp: "2026-12-29T10:00:00+01:00".to_string(),
                    capacity: 2,
                },
            ]
        );
        assert_eq!(state.notified_slots.len(), 2);
        assert_eq!(state.recent_checks[0].available_slots.len(), 4);

        let second_cycle_result = watcher
            .execute_polling_cycle(&test_config(), &test_secrets(), &mut state)
            .await;
        second_cycle_result.unwrap();
        assert_eq!(notifier.dispatched_batches.borrow().len(), 1);
        assert_eq!(state.recent_checks.len(), 2);
    }

    #[tokio::test]
    async fn does_not_mark_slots_notified_when_alert_dispatch_fails() {
        let mut calendars = HashMap::new();
        calendars.insert(
            "2026-12-01".to_string(),
            CalendarResponse {
                data: HashMap::from([("2026-12-29".to_string(), true)]),
            },
        );
        let capacities = HashMap::from([("2026-12-29T09:30:00+01:00".to_string(), 3)]);
        let mut quotas = HashMap::new();
        quotas.insert(
            "2026-12-29".to_string(),
            QuotasResponse {
                quotas: HashMap::from([("24".to_string(), QuotaEntry { capacities })]),
            },
        );

        let provider = MockAvailabilityProvider { calendars, quotas };
        let notifier = MockAlertNotifier::failing();
        let repo = InMemoryStateRepository::new();
        let mut state = StateFile::default();

        let watcher = PollingTicketWatcher::new(&provider, &notifier, &repo);

        let result = watcher
            .execute_polling_cycle(&test_config(), &test_secrets(), &mut state)
            .await;

        assert!(result.is_err());
        assert!(state.notified_slots.is_empty());
        assert_eq!(state.recent_checks.len(), 1);
    }
}
