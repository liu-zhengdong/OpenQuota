use super::freshness::{self, RefreshOutcome};
use super::SHORT_WINDOW_MAX_PERIOD_SECONDS;
use crate::{
    models::{ProviderSnapshot, QuotaFormat, QuotaWindow},
    pacing,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

/// Bound each exported collection; overflow is reported, never truncated.
pub(super) const EXPORT_LIMIT: usize = 64;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct PaceRow {
    pub(super) provider_id: String,
    pub(super) plan: Option<String>,
    pub(super) quotas: Vec<QuotaWindow>,
    pub(super) value_metrics: Vec<crate::models::ValueMetric>,
    pub(super) status_metrics: Vec<crate::models::StatusMetric>,
    pub(super) notices: Vec<crate::models::ProviderNotice>,
    pub(super) warnings: Vec<crate::models::ProviderMessage>,
    pub(super) account_identity: Option<crate::models::AccountIdentity>,
    pub(super) shared_scope: Option<crate::models::SharedScope>,
    pub(super) remembered: bool,
    pub(super) last_attempt_at: Option<DateTime<Utc>>,
    pub(super) error_kind: Option<crate::models::ProviderErrorKind>,
    pub(super) cache_identity_match: crate::providers::identity::CacheIdentityMatch,
    pub(super) refresh_outcome: String,
    pub(super) quota_count: usize,
    pub(super) quota_limit: usize,
    pub(super) value_metric_count: usize,
    pub(super) value_metric_limit: usize,
    pub(super) data_quality: String,

    pub(super) window_id: Option<String>,
    pub(super) window_label: Option<String>,
    pub(super) used_percent: Option<f64>,
    pub(super) period_elapsed_percent: Option<f64>,
    pub(super) spare_percent: Option<f64>,
    pub(super) hours_to_reset: Option<f64>,
    pub(super) short_window_id: Option<String>,
    pub(super) short_window_used_percent: Option<f64>,
    pub(super) refreshed_at: String,
    pub(super) refreshed_hours_ago: f64,
    /// The row is older than the staleness the desktop panel marks, or a requested pull
    /// failed, so its numbers are a remembered last read rather than a current one.
    pub(super) stale: bool,
    #[serde(skip)]
    pub(super) outcome: RefreshOutcome,
}

pub(super) fn build_row(
    snapshot: ProviderSnapshot,
    now: DateTime<Utc>,
    outcome: RefreshOutcome,
) -> PaceRow {
    let comparison = comparison_window(&snapshot.quotas);
    let short = short_window(&snapshot.quotas);
    PaceRow {
        quota_count: snapshot.quotas.len(),
        quota_limit: EXPORT_LIMIT,
        value_metric_count: snapshot.value_metrics.len(),
        value_metric_limit: EXPORT_LIMIT,
        data_quality: if outcome == RefreshOutcome::Failed {
            "refreshFailed"
        } else if snapshot.quotas.is_empty() && snapshot.value_metrics.is_empty() {
            "empty"
        } else if snapshot.remembered {
            "remembered"
        } else if freshness::is_stale(outcome, snapshot.refreshed_at, now) {
            "stale"
        } else if outcome == RefreshOutcome::Live {
            "live"
        } else {
            "cache"
        }
        .into(),
        account_identity: snapshot.account_identity,
        shared_scope: snapshot.shared_scope,
        remembered: snapshot.remembered,
        last_attempt_at: None,
        error_kind: None,
        cache_identity_match: Default::default(),
        refresh_outcome: match outcome {
            RefreshOutcome::Failed => "failed",
            RefreshOutcome::Live => "live",
            RefreshOutcome::Reused => "reused",
            RefreshOutcome::NotRequested => "notRequested",
        }
        .into(),
        provider_id: snapshot.provider_id,
        plan: snapshot.plan,
        window_id: comparison.map(|window| window.id.clone()),
        window_label: comparison.map(|window| window.label.clone()),
        used_percent: comparison.map(|window| window.used_percent),
        period_elapsed_percent: comparison
            .and_then(|window| pacing::period_elapsed_percent(window, now))
            .map(round1),
        spare_percent: comparison
            .and_then(|window| pacing::spare_percent(window, now))
            .map(round1),
        hours_to_reset: comparison
            .and_then(|window| window.resets_at)
            .map(|reset| round1(hours_between(now, reset))),
        short_window_id: short.map(|window| window.id.clone()),
        short_window_used_percent: short.map(|window| window.used_percent),
        refreshed_at: snapshot
            .refreshed_at
            .to_rfc3339_opts(SecondsFormat::AutoSi, true),
        refreshed_hours_ago: round1(hours_between(snapshot.refreshed_at, now).max(0.0)),
        stale: freshness::is_stale(outcome, snapshot.refreshed_at, now),
        outcome,
        quotas: snapshot.quotas,
        value_metrics: snapshot.value_metrics,
        status_metrics: snapshot.status_metrics,
        notices: snapshot.notices,
        warnings: snapshot.warnings,
    }
}

/// Sorts by spare percentage, largest first, keeping providers without a
/// comparable window at the end in their original order.
pub(super) fn sort_rows(rows: &mut [PaceRow]) {
    rows.sort_by(
        |left, right| match (left.spare_percent, right.spare_percent) {
            (Some(left_spare), Some(right_spare)) => right_spare
                .partial_cmp(&left_spare)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        },
    );
}

pub(super) fn comparison_window(quotas: &[QuotaWindow]) -> Option<&QuotaWindow> {
    let percent = percent_windows(quotas).collect::<Vec<_>>();
    let weekly = percent
        .iter()
        .copied()
        .filter(|window| matches_name(window, "week"))
        .collect::<Vec<_>>();
    if !weekly.is_empty() {
        return longest_window(weekly);
    }
    longest_window(percent)
}

pub(super) fn short_window(quotas: &[QuotaWindow]) -> Option<&QuotaWindow> {
    let percent = percent_windows(quotas).collect::<Vec<_>>();
    if let Some(session) = percent
        .iter()
        .copied()
        .find(|window| matches_name(window, "session"))
    {
        return Some(session);
    }
    percent
        .iter()
        .copied()
        .filter(|window| {
            window.period_seconds > 0 && window.period_seconds <= SHORT_WINDOW_MAX_PERIOD_SECONDS
        })
        .reduce(|best, window| {
            if window.period_seconds < best.period_seconds {
                window
            } else {
                best
            }
        })
}

pub(super) fn percent_windows(quotas: &[QuotaWindow]) -> impl Iterator<Item = &QuotaWindow> {
    quotas
        .iter()
        .filter(|window| window.format == QuotaFormat::Percent)
}

pub(super) fn matches_name(window: &QuotaWindow, needle: &str) -> bool {
    window.id.to_ascii_lowercase().contains(needle)
        || window.label.to_ascii_lowercase().contains(needle)
}

pub(super) fn longest_window(windows: Vec<&QuotaWindow>) -> Option<&QuotaWindow> {
    windows.into_iter().reduce(|best, window| {
        if window.period_seconds > best.period_seconds {
            window
        } else {
            best
        }
    })
}

pub(super) fn hours_between(start: DateTime<Utc>, end: DateTime<Utc>) -> f64 {
    end.signed_duration_since(start).num_seconds() as f64 / 3_600.0
}

pub(super) fn round1(value: f64) -> f64 {
    // The `+ 0.0` normalizes negative zero away from JSON and text output.
    (value * 10.0).round() / 10.0 + 0.0
}
