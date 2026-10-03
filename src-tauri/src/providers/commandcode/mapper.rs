use super::{client::UsageData, CommandCodeError};
use crate::models::{
    MetricValue, MetricValueKind, ProviderSnapshot, QuotaFormat, QuotaWindow, StatusMetric,
    StatusTone, ValueMetric,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

// CLI v1.74.1 normalizes separators and matches the longest plan prefix first.
const PLANS: &[(&str, &str, f64)] = &[
    ("individual-pro-v1", "Pro", 80.0),
    ("individual-go-v1", "Go", 10.0),
    ("individual-provider", "Provider", 15.0),
    ("individual-goat", "GOAT", 70.0),
    ("individual-pro", "Pro", 30.0),
    ("individual-go", "Go", 10.0),
    ("individual-max", "Max", 150.0),
    ("individual-ultra", "Ultra", 300.0),
    ("teams-pro", "Teams Pro", 40.0),
];
fn plan(id: &str) -> Option<(&'static str, f64)> {
    let id = id.to_lowercase().replace('_', "-");
    PLANS
        .iter()
        .find(|(prefix, _, _)| id.starts_with(prefix))
        .map(|(_, name, pool)| (*name, *pool))
}
fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
}
fn balance_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .map(|n| n.max(0.0))
}
fn date(value: Option<&Value>) -> Option<DateTime<Utc>> {
    if let Some(ms) = value.and_then(Value::as_i64) {
        if ms <= 0 {
            return None;
        }
        return DateTime::from_timestamp_millis(ms);
    }
    value
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}
fn dollars(id: &str, label: &str, n: f64) -> ValueMetric {
    ValueMetric {
        id: id.into(),
        label: label.into(),
        values: vec![MetricValue {
            number: n,
            kind: MetricValueKind::Dollars,
            label: None,
            estimated: false,
        }],
        expiries_at: vec![],
    }
}
fn quota(
    id: &str,
    label: &str,
    used: f64,
    cap: f64,
    reset: Option<DateTime<Utc>>,
    seconds: u64,
    format: QuotaFormat,
) -> QuotaWindow {
    QuotaWindow {
        id: id.into(),
        label: label.into(),
        used_percent: if cap > 0.0 {
            (used / cap * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        },
        resets_at: reset,
        period_seconds: seconds,
        format,
        used_value: Some(used),
        limit_value: Some(cap),
        unit: None,
        estimated: false,
        source_note: None,
    }
}
pub fn map_usage(
    data: &UsageData,
    now: DateTime<Utc>,
) -> Result<ProviderSnapshot, CommandCodeError> {
    let envelope = &data.credits;
    let credits = envelope
        .get("credits")
        .ok_or(CommandCodeError::InvalidResponse)?;
    let monthly =
        balance_number(credits.get("monthlyCredits")).ok_or(CommandCodeError::InvalidResponse)?;
    let purchased =
        balance_number(credits.get("purchasedCredits")).ok_or(CommandCodeError::InvalidResponse)?;
    let free =
        balance_number(credits.get("freeCredits")).ok_or(CommandCodeError::InvalidResponse)?;
    let subscription = data
        .subscription
        .get("data")
        .ok_or(CommandCodeError::InvalidResponse)?;
    let active = matches!(
        subscription.get("status").and_then(Value::as_str),
        Some("active")
    );
    let plan_id = subscription
        .get("planId")
        .and_then(Value::as_str)
        .or_else(|| credits.get("planId").and_then(Value::as_str));
    let info = plan_id.and_then(plan);
    let plan_name = info
        .map(|(name, _)| name.to_owned())
        .or_else(|| plan_id.map(str::to_owned))
        .or_else(|| (!active).then(|| "Free".into()));
    let spent =
        balance_number(data.summary.get("totalCost")).ok_or(CommandCodeError::InvalidResponse)?;
    let remaining = monthly + purchased + free;
    let pool = if active {
        info.map(|(_, pool)| pool.max(monthly) + purchased + free)
            .unwrap_or(spent + remaining)
    } else {
        spent + remaining
    };
    if !remaining.is_finite() || !pool.is_finite() {
        return Err(CommandCodeError::InvalidResponse);
    }
    let end = date(subscription.get("currentPeriodEnd"));
    let start = date(subscription.get("currentPeriodStart"));
    let period = start
        .zip(end)
        .filter(|(s, e)| e > s)
        .map(|(s, e)| (e - s).num_seconds() as u64)
        .unwrap_or(0);
    let mut quotas = vec![quota(
        "credits",
        "Credits",
        pool - remaining,
        pool,
        end,
        period,
        QuotaFormat::Dollars,
    )];
    quotas.extend(window_quotas(envelope));
    let statuses = renewal_status(end, now);
    Ok(ProviderSnapshot {
        provider_id: "commandcode".into(),
        plan: plan_name,
        quotas,
        value_metrics: vec![
            dollars("remaining", "Total remaining", remaining),
            dollars("freeCredits", "Free credits", free),
            dollars("monthlyCredits", "Monthly remaining", monthly),
            dollars("purchasedCredits", "Purchased credits", purchased),
        ],
        status_metrics: statuses,
        notices: vec![],
        warnings: vec![],
        refreshed_at: now,
        remembered: false,
    })
}

fn window_quotas(envelope: &Value) -> Vec<QuotaWindow> {
    let mut quotas = Vec::new();
    // CLI's usage view reads windowLimits from the outer credits envelope.
    // Unknown or incomplete windows are omitted, never inferred from plan or orgLimits.
    if let Some(windows) = envelope
        .get("windowLimits")
        .filter(|w| w.get("limited").and_then(Value::as_bool) == Some(true))
    {
        for (id, label, seconds) in [
            ("fiveHour", "5-hour", 5 * 3600),
            ("weekly", "Weekly", 7 * 86400),
        ] {
            if let Some(window) = windows.get(id) {
                if let (Some(used), Some(cap)) = (
                    number(window.get("used")),
                    number(window.get("cap")).filter(|cap| *cap > 0.0),
                ) {
                    quotas.push(quota(
                        id,
                        label,
                        used,
                        cap,
                        date(window.get("resetAt")),
                        seconds,
                        QuotaFormat::Percent,
                    ));
                }
            }
        }
    }
    quotas
}

fn renewal_status(end: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Vec<StatusMetric> {
    let mut statuses = vec![];
    if let Some(end) = end {
        let days = ((end - now).num_milliseconds() as f64 / 86_400_000.0)
            .ceil()
            .max(0.0) as i64;
        statuses.push(StatusMetric {
            id: "daysLeft".into(),
            label: "Days to renewal".into(),
            text: days.to_string(),
            tone: if days < 3 {
                StatusTone::Danger
            } else if days < 7 {
                StatusTone::Warning
            } else {
                StatusTone::Neutral
            },
            subtitle: None,
        });
    }
    statuses
}
