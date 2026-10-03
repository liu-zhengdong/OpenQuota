use super::{client::UsageData, CommandCodeError};
use crate::models::{
    MetricValue, MetricValueKind, ProviderSnapshot, QuotaFormat, QuotaWindow, StatusMetric,
    StatusTone, ValueMetric,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

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
        remaining_value: None,
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
    let plan_id = subscription
        .get("planId")
        .and_then(Value::as_str)
        .or_else(|| credits.get("planId").and_then(Value::as_str));
    let plan_name = plan_id.map(str::to_owned);
    let remaining = monthly + purchased + free;
    if !remaining.is_finite() {
        return Err(CommandCodeError::InvalidResponse);
    }
    let end = date(subscription.get("currentPeriodEnd"));
    let quotas = window_quotas(envelope);
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
        account_identity: None,
        shared_scope: None,
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
