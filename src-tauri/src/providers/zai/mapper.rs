use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::models::{QuotaFormat, QuotaWindow};

use super::ZaiError;

pub const MONTHLY_PERIOD_SECONDS: u64 = 30 * 24 * 60 * 60;

pub fn is_no_coding_plan(body: &Value) -> bool {
    body.get("success").and_then(Value::as_bool) == Some(false)
        && body
            .get("msg")
            .and_then(Value::as_str)
            .is_some_and(|message| message.to_ascii_lowercase().contains("coding plan"))
}

pub fn plan_name(body: &Value) -> Option<String> {
    successful_envelope(body).ok()?;
    body.get("data")
        .and_then(Value::as_array)?
        .first()?
        .get("productName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

// Older bare payloads have no envelope. An explicit failure is never an empty success.
fn successful_envelope(body: &Value) -> Result<(), ZaiError> {
    if body
        .get("success")
        .is_some_and(|v| v.as_bool() != Some(true))
        || body
            .get("code")
            .is_some_and(|v| number(Some(v)) != Some(200.0))
    {
        return Err(ZaiError::InvalidResponse);
    }
    Ok(())
}

pub fn map_quota(body: &Value) -> Result<Vec<QuotaWindow>, ZaiError> {
    successful_envelope(body)?;
    let root = body.as_object().ok_or(ZaiError::InvalidResponse)?;
    let container = match root.get("data") {
        Some(data) => data.as_object().ok_or(ZaiError::InvalidResponse)?,
        None => root,
    };
    let limits = container
        .get("limits")
        .and_then(Value::as_array)
        .ok_or(ZaiError::InvalidResponse)?;
    if limits.is_empty() {
        return Ok(Vec::new());
    }
    let limits = limits
        .iter()
        .map(|entry| entry.as_object().ok_or(ZaiError::InvalidResponse))
        .collect::<Result<Vec<_>, _>>()?;

    let mut session = None;
    let mut weekly = None;
    let mut mcp_usage = None;
    for entry in limits.iter().copied() {
        if matches_limit(entry, "TIME_LIMIT") {
            if mcp_usage.replace(mcp_quota(entry)?).is_some() {
                return Err(ZaiError::InvalidResponse);
            }
            continue;
        }
        let credit = matches_limit(entry, "CREDIT_LIMIT");
        if !credit && !matches_limit(entry, "TOKENS_LIMIT") {
            return Err(ZaiError::InvalidResponse);
        }
        let window = if credit {
            credit_window(entry)?
        } else {
            classify_token_window(entry)?.ok_or(ZaiError::InvalidResponse)?
        };
        let mut quota = percent_quota(entry, window)?;
        if credit {
            quota.format = QuotaFormat::Count;
            quota.used_value = source_amount(entry, "currentValue")?;
            quota.limit_value = source_amount(entry, "usage")?;
            quota.remaining_value = source_amount(entry, "remaining")?;
            quota.unit = Some("credits".into());
            quota.source_note = Some("data.limits.CREDIT_LIMIT: Z.ai Coding Plan credits, not tokens or API balance; remaining only when supplied".into());
        }
        let target = match window.kind {
            TokenWindowKind::Session => &mut session,
            TokenWindowKind::Weekly => &mut weekly,
        };
        if target.replace(quota).is_some() {
            return Err(ZaiError::InvalidResponse);
        }
    }

    Ok(session.into_iter().chain(weekly).chain(mcp_usage).collect())
}

#[derive(Debug, Clone, Copy)]
enum TokenWindowKind {
    Session,
    Weekly,
}

#[derive(Debug, Clone, Copy)]
struct TokenWindow {
    kind: TokenWindowKind,
    period_seconds: u64,
}

fn classify_token_window(
    entry: &serde_json::Map<String, Value>,
) -> Result<Option<TokenWindow>, ZaiError> {
    let unit = number(entry.get("unit")).ok_or(ZaiError::InvalidResponse)?;
    let count = number(entry.get("number"))
        .filter(|value| *value > 0.0)
        .ok_or(ZaiError::InvalidResponse)?;
    let unit_seconds = match unit {
        3.0 => 60.0 * 60.0,
        4.0 => 24.0 * 60.0 * 60.0,
        5.0 => MONTHLY_PERIOD_SECONDS as f64,
        6.0 => 7.0 * 24.0 * 60.0 * 60.0,
        _ => return Ok(None),
    };
    let duration = unit_seconds * count;
    if !duration.is_finite() || duration < 1.0 || duration > u64::MAX as f64 {
        return Err(ZaiError::InvalidResponse);
    }
    let period_seconds = duration.trunc() as u64;
    Ok(Some(TokenWindow {
        kind: if period_seconds < 24 * 60 * 60 {
            TokenWindowKind::Session
        } else {
            TokenWindowKind::Weekly
        },
        period_seconds,
    }))
}

// The official international console selects CREDIT_LIMIT by unit 3 (5h) / 6 (weekly).
// Never substitute a plan's advertised capacity for response measurements.
fn credit_window(entry: &serde_json::Map<String, Value>) -> Result<TokenWindow, ZaiError> {
    let (kind, count, period_seconds) = match number(entry.get("unit")) {
        Some(3.0) => (TokenWindowKind::Session, 5.0, 5 * 60 * 60),
        Some(6.0) => (TokenWindowKind::Weekly, 1.0, 7 * 24 * 60 * 60),
        _ => return Err(ZaiError::InvalidResponse),
    };
    if entry
        .get("number")
        .is_some_and(|v| number(Some(v)) != Some(count))
    {
        return Err(ZaiError::InvalidResponse);
    }
    Ok(TokenWindow {
        kind,
        period_seconds,
    })
}

fn source_amount(
    entry: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<f64>, ZaiError> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(None),
        value => number(value)
            .filter(|value| *value >= 0.0)
            .map(Some)
            .ok_or(ZaiError::InvalidResponse),
    }
}

fn percent_quota(
    entry: &serde_json::Map<String, Value>,
    window: TokenWindow,
) -> Result<QuotaWindow, ZaiError> {
    let used_percent = number(entry.get("percentage")).ok_or(ZaiError::InvalidResponse)?;
    let (id, label) = match window.kind {
        TokenWindowKind::Session => ("session", "Session"),
        TokenWindowKind::Weekly => ("weekly", "Weekly"),
    };
    Ok(QuotaWindow {
        id: id.into(),
        label: label.into(),
        used_percent,
        resets_at: reset_time(entry.get("nextResetTime")),
        period_seconds: window.period_seconds,
        format: QuotaFormat::Percent,
        used_value: None,
        limit_value: None,
        remaining_value: None,
        unit: None,
        estimated: false,
        source_note: None,
    })
}

fn mcp_quota(entry: &serde_json::Map<String, Value>) -> Result<QuotaWindow, ZaiError> {
    let used = number(entry.get("currentValue"))
        .filter(|value| *value >= 0.0)
        .ok_or(ZaiError::InvalidResponse)?;
    let limit = number(entry.get("usage"))
        .filter(|value| *value >= 0.0)
        .ok_or(ZaiError::InvalidResponse)?;
    Ok(QuotaWindow {
        id: "webSearches".into(),
        label: "MCP Usage".into(),
        used_percent: number(entry.get("percentage")).ok_or(ZaiError::InvalidResponse)?,
        resets_at: reset_time(entry.get("nextResetTime")),
        period_seconds: 0,
        format: QuotaFormat::Count,
        used_value: Some(used),
        limit_value: Some(limit),
        remaining_value: source_amount(entry, "remaining")?,
        unit: None,
        estimated: false,
        source_note: Some(
            format!("data.limits.TIME_LIMIT: MCP monthly usage; count unit and calendar seconds unknown; source unit={}, number={}", entry.get("unit").unwrap_or(&Value::Null), entry.get("number").unwrap_or(&Value::Null)),
        ),
    })
}

fn matches_limit(entry: &serde_json::Map<String, Value>, expected: &str) -> bool {
    entry.get("type").and_then(Value::as_str) == Some(expected)
        || entry.get("name").and_then(Value::as_str) == Some(expected)
}

fn reset_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let milliseconds = number(value)?;
    if milliseconds < i64::MIN as f64 || milliseconds > i64::MAX as f64 {
        return None;
    }
    DateTime::from_timestamp_millis(milliseconds.trunc() as i64)
}

fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        })
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::{json, Value};

    use super::{is_no_coding_plan, map_quota, plan_name};
    use crate::{models::QuotaFormat, providers::zai::ZaiError};

    fn captured_quota() -> Value {
        serde_json::from_str(include_str!("fixtures/quota.json")).unwrap()
    }

    fn captured_subscription() -> Value {
        serde_json::from_str(include_str!("fixtures/subscription.json")).unwrap()
    }

    #[test]
    fn captured_payload_maps_session_weekly_web_searches_and_plan() {
        let quotas = map_quota(&captured_quota()).unwrap();

        assert_eq!(
            plan_name(&captured_subscription()).as_deref(),
            Some("GLM Coding Pro")
        );
        assert_eq!(
            quotas
                .iter()
                .map(|quota| quota.id.as_str())
                .collect::<Vec<_>>(),
            ["session", "weekly", "webSearches"]
        );
        let session = &quotas[0];
        assert_eq!(session.used_percent, 17.0);
        assert_eq!(session.period_seconds, 5 * 60 * 60);
        assert_eq!(
            session.resets_at,
            Utc.timestamp_millis_opt(1_782_724_971_179).single()
        );
        let weekly = &quotas[1];
        assert_eq!(weekly.period_seconds, 7 * 24 * 60 * 60);

        let web = &quotas[2];
        assert_eq!(web.format, QuotaFormat::Count);
        assert_eq!(web.used_value, Some(0.0));
        assert_eq!(web.limit_value, Some(1_000.0));
        assert_eq!(web.unit, None);
        assert_eq!(web.remaining_value, Some(1000.0));
        assert_eq!(web.label, "MCP Usage");
        assert_eq!(web.period_seconds, 0);
        assert!(!web.estimated);
        assert!(web.source_note.as_deref().unwrap().contains("TIME_LIMIT"));
    }

    #[test]
    fn mcp_source_precision_remaining_and_unknown_unit_survive() {
        let mapped = map_quota(&json!({"data":{"limits":[
            {"type":"TIME_LIMIT","currentValue":1.23456789,"usage":100,
             "remaining":0,"percentage":25.123456789}
        ]}}))
        .unwrap();
        assert_eq!(mapped[0].used_percent, 25.123456789);
        assert_eq!(mapped[0].remaining_value, Some(0.0));
        assert_eq!(mapped[0].used_value, Some(1.23456789));
        assert_eq!(mapped[0].unit, None);
        assert_eq!(mapped[0].resets_at, None);
    }

    #[test]
    fn payload_windows_drive_classification_and_percentages_retain_source_precision() {
        let mapped = map_quota(&json!({"data":{"limits":[
            {"type":"TOKENS_LIMIT","unit":3,"number":3,"percentage":10.123456789},
            {"type":"TOKENS_LIMIT","unit":4,"number":3,"percentage":"99.123456789"}
        ]}}))
        .unwrap();

        assert_eq!(mapped[0].id, "session");
        assert_eq!(mapped[0].period_seconds, 3 * 60 * 60);
        assert_eq!(mapped[0].used_percent, 10.123456789);
        assert_eq!(mapped[1].id, "weekly");
        assert_eq!(mapped[1].period_seconds, 3 * 24 * 60 * 60);
        assert_eq!(mapped[1].used_percent, 99.123456789);
    }

    #[test]
    fn explicit_zeroes_are_measurements_instead_of_missing_data() {
        let mapped = map_quota(&json!({"limits":[
            {"name":"TOKENS_LIMIT","unit":"3","number":"5","percentage":0},
            {"name":"TIME_LIMIT","currentValue":"0","usage":"0","percentage":0}
        ]}))
        .unwrap();

        assert_eq!(mapped[0].used_percent, 0.0);
        assert_eq!(mapped[1].used_value, Some(0.0));
        assert_eq!(mapped[1].limit_value, Some(0.0));
        assert_eq!(mapped[1].used_percent, 0.0);
    }

    #[test]
    fn malformed_envelopes_and_partial_known_limits_fail_loudly() {
        for body in [
            Value::Null,
            json!({"data":[]}),
            json!({"data":{}}),
            json!({"data":{"limits":{}}}),
            json!({"data":{"limits":[null]}}),
            json!({"data":{"limits":[{"type":"TOKENS_LIMIT","unit":3,"number":5}]}}),
            json!({"data":{"limits":[{"type":"TOKENS_LIMIT","unit":3,"number":0,"percentage":5}]}}),
            json!({"data":{"limits":[{"type":"TIME_LIMIT","usage":1000}]}}),
            json!({"data":{"limits":[{"type":"TIME_LIMIT","currentValue":0}]}}),
            json!({"data":{"limits":[{"type":"TIME_LIMIT","currentValue":-1,"usage":1000}]}}),
            json!({"data":{"limits":[{"type":"TIME_LIMIT","currentValue":0,"usage":0,"percentage":"bad"}]}}),
            json!({"data":{"limits":[{"type":"TIME_LIMIT","currentValue":0,"usage":0}]}}),
            json!({"data":{"limits":[{"type":"CREDIT_LIMIT","unit":3,"percentage":10}, {"type":"TIME_LIMIT","currentValue":0,"usage":0,"percentage":"bad"}]}}),
        ] {
            assert!(matches!(map_quota(&body), Err(ZaiError::InvalidResponse)));
        }
        assert!(map_quota(&json!({"data":{"limits":[]}}))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn unknown_limits_or_units_are_not_silently_successful() {
        for unknown in [
            json!({"type":"FUTURE_LIMIT"}),
            json!({"type":"TOKENS_LIMIT","unit":99,"number":1,"percentage":70}),
            json!({"type":"CREDIT_LIMIT","unit":99,"percentage":70}),
        ] {
            assert!(map_quota(&json!({"data":{"limits":[unknown]}})).is_err());
            assert!(map_quota(&json!({"data":{"limits":[unknown,
                {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":25}
            ]}}))
            .is_err());
        }
    }

    #[test]
    fn invalid_or_unknown_resets_are_omitted_without_losing_usage() {
        let mapped = map_quota(&json!({"data":{"limits":[
            {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":25,
             "nextResetTime":"not-a-time"},
            {"type":"TIME_LIMIT","currentValue":1,"usage":10,"percentage":10,
             "nextResetTime":"1785292686976"}
        ]}}))
        .unwrap();

        assert_eq!(mapped[0].resets_at, None);
        assert_eq!(
            mapped[1].resets_at,
            Utc.timestamp_millis_opt(1_785_292_686_976).single()
        );
    }

    #[test]
    fn no_plan_signal_and_optional_subscription_are_structurally_checked() {
        assert!(is_no_coding_plan(&json!({
            "success":false,
            "code":500,
            "msg":"Current user does not have a coding plan"
        })));
        assert!(!is_no_coding_plan(&json!({
            "success":false,
            "msg":"internal error"
        })));
        assert!(!is_no_coding_plan(&json!({
            "success":"false",
            "msg":"coding plan"
        })));

        assert_eq!(
            plan_name(&json!({"data":[{"productName":" GLM Coding Max "}]})).as_deref(),
            Some("GLM Coding Max")
        );
        assert_eq!(plan_name(&json!({"data":[]})), None);
        assert_eq!(plan_name(&json!({"data":[{"productName":42}]})), None);
    }

    #[test]
    fn credits_keep_source_precision_units_windows_and_null_remaining() {
        let body: Value = serde_json::from_str(include_str!("fixtures/credits.json")).unwrap();
        let quotas = map_quota(&body).unwrap();
        assert_eq!(quotas.len(), 2);
        assert_eq!(quotas[0].id, "session");
        assert_eq!(quotas[0].period_seconds, 18_000);
        assert_eq!(quotas[0].format, QuotaFormat::Count);
        assert_eq!(quotas[0].unit.as_deref(), Some("credits"));
        assert_eq!(quotas[0].used_value, Some(11952.123456));
        assert_eq!(quotas[0].limit_value, Some(12000.0));
        assert_eq!(quotas[0].remaining_value, Some(47.876544));
        assert_eq!(quotas[0].used_percent, 99.6010288);
        assert_eq!(
            quotas[0].resets_at,
            Utc.timestamp_millis_opt(1791028800000).single()
        );
        assert_eq!(quotas[1].id, "weekly");
        assert_eq!(quotas[1].period_seconds, 604800);
        assert_eq!(quotas[1].remaining_value, None);
        assert_eq!(quotas[1].resets_at, None);
        assert!(quotas.iter().all(|q| !q.estimated));
    }

    #[test]
    fn credits_do_not_invent_amounts_and_keep_zeroes() {
        let quotas = map_quota(&json!({"limits":[
            {"type":"CREDIT_LIMIT","unit":3,"percentage":100,"currentValue":12000,"usage":12000,"remaining":0},
            {"type":"CREDIT_LIMIT","unit":6,"percentage":0,"currentValue":null,"usage":null}
        ]})).unwrap();
        assert_eq!(quotas[0].remaining_value, Some(0.0));
        assert_eq!(quotas[1].used_value, None);
        assert_eq!(quotas[1].limit_value, None);
        assert_eq!(quotas[1].remaining_value, None);
        let mut body = json!({"limits":[{"type":"CREDIT_LIMIT","unit":3,"percentage":25,"currentValue":40,"usage":100,"remaining":50}]});
        assert_eq!(map_quota(&body).unwrap()[0].remaining_value, Some(50.0));
        for bad in [json!(-1), json!("NaN"), json!("bad"), json!({})] {
            body["limits"][0]["remaining"] = bad;
            assert!(map_quota(&body).is_err());
        }
        body["limits"][0]["remaining"] = Value::Null;
        body["limits"][0]["number"] = json!(24);
        assert!(map_quota(&body).is_err());
    }

    #[test]
    fn explicit_failure_envelopes_never_become_fresh_empty_usage_or_plan_names() {
        for (success, code) in [
            (json!(false), json!(500)),
            (json!(true), json!(500)),
            (json!("true"), json!(200)),
        ] {
            let body =
                json!({"success":success,"code":code,"msg":"internal error","data":{"limits":[]}});
            assert_eq!(map_quota(&body), Err(ZaiError::InvalidResponse));
            let subscription =
                json!({"success":success,"code":code,"data":[{"productName":"GLM Coding Pro"}]});
            assert_eq!(plan_name(&subscription), None);
        }
    }

    #[test]
    fn duplicate_known_windows_are_rejected_before_snapshot_validation() {
        let error = map_quota(&json!({"data":{"limits":[
            {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":10},
            {"type":"TOKENS_LIMIT","unit":3,"number":3,"percentage":20}
        ]}}))
        .unwrap_err();

        assert!(matches!(error, ZaiError::InvalidResponse));
    }
}
