use super::invalid;
use crate::{
    models::{AccountIdentity, ProviderSnapshot, QuotaFormat, QuotaWindow},
    providers::ProviderError,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub(super) struct Quota {
    pub provider: String,
    pub kind: String,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub user: String,
    pub windows: Vec<Window>,
    #[serde(default)]
    pub error: String,
    #[serde(default, rename = "asOf")]
    pub as_of: Option<DateTime<Utc>>,
}
#[derive(Clone, Deserialize)]
pub(super) struct Window {
    pub name: String,
    pub used: f64,
    pub remaining: f64,
    #[serde(default, rename = "resetsAt")]
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unlimited: bool,
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub limit: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
}

fn family(provider: &str) -> Option<&str> {
    match provider {
        "codex" => Some("codex"),
        "grok" | "grok-plugin" => Some("grok"),
        "kimi-code" | "kimi-code-cn" | "kimi" => Some("kimi"),
        "zai" | "z-ai" | "zhipu" | "zcode" | "zcode-plugin" => Some("zai"),
        "opencode-go" => Some("opencode"),
        _ => None,
    }
}
// Never average accounts or silently pick one: the app's existing provider ID is one account.
pub(super) fn select<'a>(report: &'a [Quota], id: &str) -> Result<&'a Quota, ProviderError> {
    let mut candidates = report.iter().filter(|q| {
        family(&q.provider) == Some(id) && matches!(q.kind.as_str(), "plan" | "subscription")
    });
    let quota = candidates
        .next()
        .ok_or_else(|| invalid("Magpie has no quota for this provider."))?;
    if candidates.next().is_some() {
        return Err(invalid(
            "Magpie has multiple accounts for this provider; account selection is required.",
        ));
    }
    if !quota.error.is_empty() {
        return Err(invalid("Magpie could not read this provider's quota."));
    }
    if quota.windows.is_empty() {
        return Err(invalid("Magpie returned no quota windows."));
    }
    Ok(quota)
}

fn window_source(provider: &str, name: &str) -> (String, u64) {
    let known = match name {
        "5 hours" => Some(("session", 18_000)),
        "7 days" => Some(("weekly", 604_800)),
        "Month" if provider == "opencode" => Some(("monthly", 2_592_000)),
        "MCP · Month" if provider == "zai" => Some(("webSearches", 2_592_000)),
        _ => None,
    };
    if let Some((id, seconds)) = known {
        return (id.into(), seconds);
    }
    // Unknown named windows retain their own metric, using the existing scoped-quota path.
    (
        format!("magpie-{}", crate::hashing::sha256_hex(name.as_bytes())),
        0,
    )
}

pub(super) fn snapshot(quota: &Quota, id: &str) -> Result<ProviderSnapshot, ProviderError> {
    let mut quotas = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for window in &quota.windows {
        if window.name.trim().is_empty()
            || window.unlimited
            || !window.used.is_finite()
            || window.used < 0.0
            || !window.remaining.is_finite()
            || !(0.0..=100.0).contains(&window.remaining)
            || (window.remaining - (100.0 - window.used).max(0.0)).abs() > 1e-7
        {
            return Err(invalid(
                "Magpie returned an invalid or unlimited quota window.",
            ));
        }
        let (source, period_seconds) = window_source(id, &window.name);
        if !seen.insert(source.clone()) {
            return Err(invalid("Magpie returned duplicate quota windows."));
        }
        let counted = window.unit.is_some() && window.amount.is_some() && window.limit.is_some();
        if counted
            && (!window.amount.is_some_and(|n| n.is_finite() && n >= 0.0)
                || !window.limit.is_some_and(|n| n.is_finite() && n > 0.0))
        {
            return Err(invalid("Magpie returned invalid quota counts."));
        }
        quotas.push(QuotaWindow {
            id: source,
            label: window.name.clone(),
            used_percent: window.used,
            resets_at: window.resets_at,
            period_seconds,
            format: if counted {
                QuotaFormat::Count
            } else {
                QuotaFormat::Percent
            },
            used_value: counted.then_some(window.amount).flatten(),
            limit_value: counted.then_some(window.limit).flatten(),
            remaining_value: None,
            unit: counted.then_some(window.unit.clone()).flatten(),
            estimated: false,
            source_note: Some("magpie".into()),
        });
    }
    Ok(ProviderSnapshot {
        provider_id: id.into(),
        plan: quota.plan.clone(),
        quotas,
        value_metrics: vec![],
        status_metrics: vec![],
        notices: vec![],
        warnings: vec![],
        refreshed_at: quota.as_of.unwrap_or_else(Utc::now),
        remembered: quota.as_of.is_some(),
        account_identity: (!quota.user.is_empty()).then(|| AccountIdentity {
            kind: "magpie-account".into(),
            value: crate::hashing::sha256_hex(
                format!("{}/{}", quota.provider, quota.user).as_bytes(),
            ),
            source: "magpie".into(),
        }),
        shared_scope: None,
    })
}
