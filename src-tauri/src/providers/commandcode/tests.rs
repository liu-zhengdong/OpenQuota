use super::{
    auth::CommandCodeAuth,
    client::{CommandCodeClient, UsageData},
    mapper::map_usage,
    CommandCodeProvider,
};
use crate::{
    models::{ProviderErrorKind, StatusTone},
    providers::{test_http, ProviderRegistry, UsageProvider},
};
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
const CREDITS: &str = include_str!("fixtures/credits.json");
const SUBSCRIPTION: &str = include_str!("fixtures/subscription.json");
fn fixture() -> UsageData {
    UsageData {
        credits: serde_json::from_str(CREDITS).unwrap(),
        subscription: serde_json::from_str(SUBSCRIPTION).unwrap(),
    }
}
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap()
}

#[test]
fn subscription_remaining_free_and_windows_are_independent() {
    let snapshot = map_usage(&fixture(), now()).unwrap();
    assert_eq!(snapshot.plan.as_deref(), Some("individual-pro"));
    assert_eq!(
        snapshot
            .value_metrics
            .iter()
            .map(|v| v.values[0].number)
            .collect::<Vec<_>>(),
        [20.0, 5.0, 12.0, 3.0]
    );
    assert_eq!(snapshot.quotas.len(), 2);
    assert!(snapshot.quotas.iter().all(|q| q.id != "credits"));
    assert_eq!(snapshot.quotas[0].used_percent, 20.0);
    assert_eq!(snapshot.quotas[1].used_percent, 40.0);
    assert_eq!(
        snapshot.quotas[0].resets_at.unwrap().timestamp_millis(),
        1791018000000
    );
    assert_eq!(snapshot.status_metrics[0].text, "29");
}
#[test]
fn free_account_zero_balance_and_unknown_windows_do_not_invent_data() {
    let mut data = fixture();
    data.subscription = json!({"data":null});
    data.credits = json!({"credits":{"monthlyCredits":0,"purchasedCredits":0,"freeCredits":2},"windowLimits":{"unknown":true}});
    let snapshot = map_usage(&data, now()).unwrap();
    assert_eq!(snapshot.plan, None);
    assert!(snapshot.quotas.is_empty());
    assert!(snapshot.status_metrics.is_empty());
    data.credits["credits"]["freeCredits"] = json!(0);
    assert_eq!(
        map_usage(&data, now()).unwrap().value_metrics[1].values[0].number,
        0.0
    );
}
#[test]
fn malformed_credit_fields_are_rejected() {
    for value in [Value::Null, json!("12"), json!({})] {
        let mut data = fixture();
        data.credits["credits"]["freeCredits"] = value;
        assert!(map_usage(&data, now()).is_err());
    }
}
#[test]
fn renewal_urgency_and_bad_windows() {
    for (end, days, tone) in [
        ("2026-10-02T00:00:00Z", "0", StatusTone::Danger),
        ("2026-10-05T12:00:00Z", "3", StatusTone::Warning),
        ("2026-10-10T00:00:00Z", "7", StatusTone::Neutral),
    ] {
        let mut data = fixture();
        data.subscription["data"]["currentPeriodEnd"] = json!(end);
        let snapshot = map_usage(&data, now()).unwrap();
        assert_eq!(snapshot.status_metrics[0].text, days);
        assert_eq!(snapshot.status_metrics[0].tone, tone);
    }
    let mut data = fixture();
    data.credits["windowLimits"]["fiveHour"]["cap"] = json!(0);
    data.credits["windowLimits"]["weekly"]["used"] = json!(null);
    assert!(map_usage(&data, now()).unwrap().quotas.is_empty());
}
#[test]
fn unset_window_reset_does_not_become_the_unix_epoch() {
    let mut data = fixture();
    data.credits["windowLimits"]["fiveHour"]["resetAt"] = json!(0);
    data.credits["windowLimits"]["weekly"]["resetAt"] = json!(-1);
    let snapshot = map_usage(&data, now()).unwrap();
    assert_eq!(snapshot.quotas.len(), 2);
    assert!(snapshot.quotas[0].resets_at.is_none());
    assert!(snapshot.quotas[1].resets_at.is_none());
}

#[test]
fn authentication_rate_limit_network_and_invalid_json_are_safe() {
    for (status, kind) in [
        (401, ProviderErrorKind::Authentication),
        (403, ProviderErrorKind::Authentication),
        (429, ProviderErrorKind::RateLimited),
        (529, ProviderErrorKind::RateLimited),
        (500, ProviderErrorKind::InvalidResponse),
    ] {
        let client = CommandCodeClient::with_base(
            &test_http::serve_once(status, &[], "{}"),
            Duration::from_secs(1),
        )
        .unwrap();
        let error: crate::providers::ProviderError =
            client.fetch("fake-test-secret").err().unwrap().into();
        assert_eq!(error.kind(), kind);
        assert!(!error.to_string().contains("fake-test-secret"));
    }
    let url = test_http::serve_once_after(test_http::TIMEOUT_TEST_RESPONSE_DELAY, 200, &[], "{}");
    let client = CommandCodeClient::with_base(&url, test_http::TIMEOUT_TEST_CLIENT_LIMIT).unwrap();
    let error: crate::providers::ProviderError =
        client.fetch("fake-test-secret").err().unwrap().into();
    assert_eq!(error.kind(), ProviderErrorKind::Network);
    let client = CommandCodeClient::with_base(
        &test_http::serve_once(200, &[], "broken"),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(client.fetch("fake-test-secret").is_err());
}
#[test]
fn mock_server_checks_routes_bearer_and_org() {
    check_routes(true);
}
#[test]
fn personal_account_omits_org_id() {
    check_routes(false);
}
#[test]
fn whoami_without_an_account_is_rejected() {
    let client = CommandCodeClient::with_base(
        &test_http::serve_once(200, &[], r#"{"org":null,"user":null}"#),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(
        client.fetch("fake-test-secret"),
        Err(super::CommandCodeError::InvalidResponse)
    ));
}
fn check_routes(has_org: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 8192];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let body = if request.contains("/alpha/whoami?") {
                if has_org {
                    r#"{"org":{"id":"org/test"}}"#
                } else {
                    r#"{"org":null,"user":{"id":"user/test"}}"#
                }
            } else if request.contains("/alpha/billing/credits?") {
                CREDITS
            } else if request.contains("/alpha/billing/subscriptions?") {
                SUBSCRIPTION
            } else {
                panic!("unexpected endpoint: {request}")
            };
            captured.lock().unwrap().push(request);
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    let data = CommandCodeClient::with_base(&base, Duration::from_secs(1))
        .unwrap()
        .fetch("fake-test-secret")
        .unwrap();
    assert_eq!(
        map_usage(&data, now()).unwrap().plan.as_deref(),
        Some("individual-pro")
    );
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /alpha/whoami?limits=1 "));
    for request in requests.iter() {
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fake-test-secret"));
    }
    for request in &requests[1..] {
        assert_eq!(request.contains("orgId=org%2Ftest"), has_org);
        if !has_org {
            assert!(!request.contains("orgId="));
        }
    }
    assert_eq!(requests.len(), 3);
}
#[test]
fn unconfigured_provider_is_registered_without_own_credential_storage() {
    let dir = tempfile::tempdir().unwrap();
    let provider = CommandCodeProvider {
        auth: CommandCodeAuth::at(dir.path().join("missing.json")),
        client: CommandCodeClient::with_base("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    };
    assert!(provider.auth.load_with_env(None).unwrap().is_none());
    assert!(!provider.supports_api_key_configuration());
    let registry = ProviderRegistry::from_definitions(vec![
        crate::providers::codex::definition(),
        provider.definition(),
    ])
    .unwrap();
    assert!(registry.definition("commandcode").is_some());
    for id in ["remaining", "freeCredits", "fiveHour", "weekly", "daysLeft"] {
        assert!(registry.metric(&format!("commandcode.{id}")).is_some());
    }
    assert!(!registry
        .catalog()
        .api_key_provider_ids
        .contains(&"commandcode".into()));
}

#[test]
fn isolated_panel_and_pace_artifacts() {
    assert_eq!(
        serde_json::to_value(super::definition()).unwrap(),
        serde_json::from_str::<Value>(include_str!("fixtures/definition.json")).unwrap()
    );
    assert_eq!(
        serde_json::to_value(map_usage(&fixture(), now()).unwrap()).unwrap(),
        serde_json::from_str::<Value>(include_str!("fixtures/snapshot.json")).unwrap()
    );
    let directory = std::env::var_os("COMMANDCODE_TEST_OUTPUT");
    let Some(directory) = directory else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let snapshot = map_usage(&fixture(), now()).unwrap();
    std::fs::write(
        directory.join("snapshot.json"),
        serde_json::to_string_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("definition.json"),
        serde_json::to_string_pretty(&super::definition()).unwrap(),
    )
    .unwrap();
    let storage = crate::storage::Storage::open(&directory.join("openquota.db")).unwrap();
    storage.save_snapshot(&snapshot).unwrap();
}

#[test]
fn absent_credentials_leave_default_panel_state_without_error() {
    let dir = tempfile::tempdir().unwrap();
    let provider = CommandCodeProvider {
        auth: CommandCodeAuth::at(dir.path().join("missing.json")),
        client: CommandCodeClient::with_base("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    };
    assert!(!provider.has_local_credentials());
    let registry =
        Arc::new(ProviderRegistry::new(vec![Arc::new(provider), Arc::new(Fallback)]).unwrap());
    let storage =
        Arc::new(crate::storage::Storage::open(&dir.path().join("openquota.db")).unwrap());
    let service = crate::service::ProviderService::new(registry, storage);
    let state = service.state();
    let state = state.providers.get("commandcode").unwrap();
    assert!(state.snapshot.is_none());
    assert!(state.error.is_none());
}

struct Fallback;
impl UsageProvider for Fallback {
    fn definition(&self) -> crate::models::ProviderDefinition {
        crate::providers::codex::definition()
    }
    fn has_local_credentials(&self) -> bool {
        false
    }
    fn refresh(&self) -> Result<crate::models::ProviderSnapshot, crate::providers::ProviderError> {
        unreachable!()
    }
}

#[test]
fn rate_limits_preserve_the_last_panel_snapshot() {
    for status in [429, 529] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, r#"{"apiKey":"fake-test-secret"}"#).unwrap();
        let provider = CommandCodeProvider {
            auth: CommandCodeAuth::at(path),
            client: CommandCodeClient::with_base(
                &test_http::serve_once(status, &[], "{}"),
                Duration::from_secs(1),
            )
            .unwrap(),
        };
        let registry =
            Arc::new(ProviderRegistry::new(vec![Arc::new(provider), Arc::new(Fallback)]).unwrap());
        let storage =
            Arc::new(crate::storage::Storage::open(&dir.path().join("openquota.db")).unwrap());
        let snapshot = map_usage(&fixture(), now()).unwrap();
        storage.save_snapshot(&snapshot).unwrap();
        let service = Arc::new(crate::service::ProviderService::new(registry, storage));
        let state = tauri::async_runtime::block_on(service.refresh("commandcode", true));
        assert_eq!(state.snapshot, Some(snapshot));
        assert_eq!(state.error_kind, Some(ProviderErrorKind::RateLimited));
    }
}

#[test]
fn negative_balances_follow_the_cli_zero_floor() {
    let mut data = fixture();
    data.credits["credits"]["monthlyCredits"] = json!(-1);
    data.credits["credits"]["freeCredits"] = json!(-2);
    let snapshot = map_usage(&data, now()).unwrap();
    assert_eq!(snapshot.value_metrics[0].values[0].number, 3.0);
    assert_eq!(snapshot.value_metrics[1].values[0].number, 0.0);
}
