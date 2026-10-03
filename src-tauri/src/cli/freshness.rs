//! Pure decisions about which readings `openquota pace` shows and how fresh they are.
//!
//! Nothing here touches the database, the network, or the clock; callers pass `now` in.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};

use crate::{i18n::UiLocale, models::ProviderSnapshot, policy::STALE_AFTER};

/// A provider read this recently, by the app or an earlier `pace --refresh`, is reused
/// instead of being pulled again.
pub const REUSE_WITHIN: Duration = Duration::seconds(60);

/// What happened to a row's reading during this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// No pull was asked for; the row shows the cached reading.
    NotRequested,
    /// Pulled from the provider during this run.
    Live,
    /// A reading from the last [`REUSE_WITHIN`] was reused instead of pulling again.
    Reused,
    /// The pull failed or timed out; the row shows the earlier reading.
    Failed,
}

impl RefreshOutcome {
    /// The reading is current as of this run rather than whatever was cached.
    pub fn is_fresh(self) -> bool {
        matches!(self, Self::Live | Self::Reused)
    }
}

/// The panel shows a provider only while it is turned on. `enabled` is `None` when the
/// panel settings are unavailable, in which case every cached provider is shown.
pub fn panel_shows(enabled: Option<&HashSet<String>>, provider_id: &str) -> bool {
    enabled.is_none_or(|enabled| enabled.contains(provider_id))
}

/// Keeps the newest reading per provider. Remembered readings only replay an earlier read,
/// so they never displace a real one.
pub fn newest_readings(
    sources: impl IntoIterator<Item = ProviderSnapshot>,
) -> HashMap<String, ProviderSnapshot> {
    let mut newest: HashMap<String, ProviderSnapshot> = HashMap::new();
    for snapshot in sources {
        let replace = match newest.get(&snapshot.provider_id) {
            None => true,
            Some(current) => match (current.remembered, snapshot.remembered) {
                (true, false) => true,
                (false, true) => false,
                _ => snapshot.refreshed_at > current.refreshed_at,
            },
        };
        if replace {
            newest.insert(snapshot.provider_id.clone(), snapshot);
        }
    }
    newest
}

/// Whether a requested refresh has to reach the provider, given its newest known reading.
pub fn needs_pull(latest: Option<&ProviderSnapshot>, now: DateTime<Utc>) -> bool {
    match latest {
        None => true,
        Some(snapshot) if snapshot.remembered => true,
        Some(snapshot) => now.signed_duration_since(snapshot.refreshed_at) >= REUSE_WITHIN,
    }
}

/// A row is stale once it passes the panel's staleness window, or when a pull was asked
/// for and did not produce a current reading.
pub fn is_stale(outcome: RefreshOutcome, refreshed_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    outcome == RefreshOutcome::Failed || now.signed_duration_since(refreshed_at) >= STALE_AFTER
}

/// Age in hours of the oldest row that nothing refreshed during this run, when the
/// application is known not to be running. `None` means there is nothing to warn about.
pub fn offline_oldest_hours(
    app_running: Option<bool>,
    rows: impl IntoIterator<Item = (RefreshOutcome, f64)>,
) -> Option<f64> {
    if app_running != Some(false) {
        return None;
    }
    rows.into_iter()
        .filter(|(outcome, _)| !outcome.is_fresh())
        .map(|(_, hours)| hours)
        .reduce(f64::max)
}

pub fn offline_notice(locale: UiLocale, oldest_hours: f64) -> String {
    match locale {
        UiLocale::Zh => format!("OpenQuota 没在运行，下列读数最久 {oldest_hours:.1} 小时前。"),
        UiLocale::En => format!(
            "OpenQuota is not running; the oldest reading below is from {oldest_hours:.1} hours ago."
        ),
    }
}

/// The text table's DATA cell. Rows nothing is refreshing while the app is closed carry `*`,
/// pointing at the note under the table.
pub fn data_label(outcome: RefreshOutcome, stale: bool, app_offline: bool) -> String {
    let label = if stale {
        "stale"
    } else if outcome.is_fresh() {
        "live"
    } else {
        "cached"
    };
    if app_offline && !outcome.is_fresh() {
        format!("{label}*")
    } else {
        label.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{DateTime, Duration, TimeZone, Utc};

    use super::{
        data_label, is_stale, needs_pull, newest_readings, offline_notice, offline_oldest_hours,
        panel_shows, RefreshOutcome, REUSE_WITHIN,
    };
    use crate::{i18n::UiLocale, models::ProviderSnapshot, policy::STALE_AFTER};

    const OUTCOMES: [RefreshOutcome; 4] = [
        RefreshOutcome::NotRequested,
        RefreshOutcome::Live,
        RefreshOutcome::Reused,
        RefreshOutcome::Failed,
    ];

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    fn reading(provider_id: &str, age: Duration, remembered: bool) -> ProviderSnapshot {
        ProviderSnapshot {
            provider_id: provider_id.into(),
            plan: Some(format!("{}s", age.num_seconds())),
            quotas: Vec::new(),
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: now() - age,
            remembered,
            account_identity: None,
            shared_scope: None,
        }
    }

    #[test]
    fn only_enabled_providers_are_shown_when_the_panel_settings_are_known() {
        let enabled = HashSet::from(["cursor".to_owned(), "claude@1234abcd".to_owned()]);
        assert!(panel_shows(Some(&enabled), "cursor"));
        assert!(panel_shows(Some(&enabled), "claude@1234abcd"));
        assert!(!panel_shows(Some(&enabled), "copilot"));
        assert!(!panel_shows(Some(&HashSet::new()), "cursor"));
        assert!(panel_shows(None, "copilot"));
    }

    #[test]
    fn the_newest_real_reading_wins_over_older_and_remembered_ones() {
        let newest = newest_readings([
            reading("cursor", Duration::hours(3), false),
            reading("cursor", Duration::minutes(1), false),
            reading("cursor", Duration::hours(1), false),
            reading("claude", Duration::hours(2), false),
            reading("claude", Duration::minutes(1), true),
            reading("kimi", Duration::minutes(5), true),
            reading("kimi", Duration::minutes(1), true),
        ]);
        assert_eq!(newest.len(), 3);
        assert_eq!(newest["cursor"].plan.as_deref(), Some("60s"));
        assert_eq!(newest["claude"].plan.as_deref(), Some("7200s"));
        assert_eq!(newest["kimi"].plan.as_deref(), Some("60s"));
    }

    #[test]
    fn a_recent_reading_is_reused_instead_of_pulled_again() {
        assert!(needs_pull(None, now()));
        let just_read = reading("cursor", REUSE_WITHIN - Duration::seconds(1), false);
        assert!(!needs_pull(Some(&just_read), now()));
        let at_limit = reading("cursor", REUSE_WITHIN, false);
        assert!(needs_pull(Some(&at_limit), now()));
        let remembered = reading("cursor", Duration::seconds(1), true);
        assert!(needs_pull(Some(&remembered), now()));
        let from_the_future = reading("cursor", Duration::seconds(-30), false);
        assert!(!needs_pull(Some(&from_the_future), now()));
    }

    #[test]
    fn staleness_follows_age_except_that_a_failed_pull_is_always_stale() {
        let recent = now() - STALE_AFTER + Duration::seconds(1);
        let old = now() - STALE_AFTER;
        for outcome in OUTCOMES {
            let failed = outcome == RefreshOutcome::Failed;
            assert_eq!(is_stale(outcome, recent, now()), failed, "{outcome:?}");
            assert!(is_stale(outcome, old, now()), "{outcome:?}");
        }
    }

    #[test]
    fn the_offline_notice_covers_only_rows_nothing_refreshed() {
        let rows = [
            (RefreshOutcome::Live, 0.0),
            (RefreshOutcome::Reused, 0.0),
            (RefreshOutcome::NotRequested, 3.5),
            (RefreshOutcome::Failed, 113.5),
        ];
        assert_eq!(offline_oldest_hours(Some(false), rows), Some(113.5));
        assert_eq!(offline_oldest_hours(Some(true), rows), None);
        assert_eq!(offline_oldest_hours(None, rows), None);
        assert_eq!(
            offline_oldest_hours(Some(false), [(RefreshOutcome::Live, 0.0)]),
            None
        );
        assert_eq!(offline_oldest_hours(Some(false), []), None);
        assert_eq!(
            offline_oldest_hours(Some(false), [(RefreshOutcome::NotRequested, 0.0)]),
            Some(0.0)
        );
    }

    #[test]
    fn the_offline_notice_follows_the_interface_language() {
        assert_eq!(
            offline_notice(UiLocale::Zh, 113.46),
            "OpenQuota 没在运行，下列读数最久 113.5 小时前。"
        );
        assert_eq!(
            offline_notice(UiLocale::En, 2.0),
            "OpenQuota is not running; the oldest reading below is from 2.0 hours ago."
        );
    }

    #[test]
    fn data_labels_cover_every_outcome_staleness_and_app_state() {
        let expected = [
            (RefreshOutcome::NotRequested, false, false, "cached"),
            (RefreshOutcome::NotRequested, true, false, "stale"),
            (RefreshOutcome::NotRequested, false, true, "cached*"),
            (RefreshOutcome::NotRequested, true, true, "stale*"),
            (RefreshOutcome::Live, false, false, "live"),
            (RefreshOutcome::Live, false, true, "live"),
            (RefreshOutcome::Live, true, false, "stale"),
            (RefreshOutcome::Live, true, true, "stale"),
            (RefreshOutcome::Reused, false, false, "live"),
            (RefreshOutcome::Reused, false, true, "live"),
            (RefreshOutcome::Reused, true, false, "stale"),
            (RefreshOutcome::Reused, true, true, "stale"),
            (RefreshOutcome::Failed, false, false, "cached"),
            (RefreshOutcome::Failed, true, false, "stale"),
            (RefreshOutcome::Failed, false, true, "cached*"),
            (RefreshOutcome::Failed, true, true, "stale*"),
        ];
        for (outcome, stale, offline, label) in expected {
            assert_eq!(
                data_label(outcome, stale, offline),
                label,
                "{outcome:?} stale={stale} offline={offline}"
            );
        }
    }
}
