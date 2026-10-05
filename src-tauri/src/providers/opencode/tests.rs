use super::{client::OpenCodeClient, paths::OpenCodePaths, OpenCodeProvider};
use crate::{models::ProviderErrorKind, providers::UsageProvider};
use chrono::{TimeZone, Utc};
use std::{fs, time::Duration};
use tempfile::tempdir;
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap()
}
fn test_client(url: &str) -> OpenCodeClient {
    OpenCodeClient::for_test(url, Duration::from_secs(10))
}
#[test]
fn account_usage_endpoint_populates_quotas_without_local_history() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("auth.json"),
        r#"{"opencode-go":{"type":"api","key":"secret-key"}}"#,
    )
    .unwrap();
    let body = serde_json::json!({
        "usage": {
            "rolling": {"percent": 31, "resetsAt": "2026-08-12T12:00:00Z", "status": "ok"},
            "weekly": {"percent": 100, "resetsAt": "2026-08-17T00:00:00Z", "status": "rate-limited"},
            "monthly": {"percent": 72, "resetsAt": "2026-09-05T00:00:00Z", "status": "ok"}
        }
    })
    .to_string();
    let url = crate::providers::test_http::serve_once(200, &[], &body);
    let provider = OpenCodeProvider::with_dependencies(
        OpenCodePaths::for_data_directory(directory.path().to_path_buf()),
        test_client(&url),
        now(),
    );

    let snapshot = provider.refresh().unwrap();

    assert_eq!(snapshot.plan.as_deref(), Some("Go"));
    assert_eq!(snapshot.quotas[0].used_percent, 31.0);
    assert_eq!(snapshot.quotas[1].used_percent, 100.0);
}

#[test]
fn local_databases_are_ignored_and_auth_errors_are_typed() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("opencode.db"),
        b"corrupt unused database",
    )
    .unwrap();
    let provider = OpenCodeProvider::with_dependencies(
        OpenCodePaths::for_data_directory(directory.path().to_path_buf()),
        test_client("http://127.0.0.1:1"),
        now(),
    );
    assert!(!provider.has_local_credentials());
    assert_eq!(
        provider.refresh().unwrap_err().kind(),
        ProviderErrorKind::Authentication
    );
    fs::write(directory.path().join("auth.json"), "invalid secret").unwrap();
    let error = provider.refresh().unwrap_err();
    assert_eq!(error.kind(), ProviderErrorKind::CredentialStorage);
    assert!(!error.to_string().contains("secret"));
}
#[test]
fn cloud_subscription_and_rate_limit_errors_are_preserved() {
    for (status, body, kind) in [
        (
            403,
            r#"{"type":"error","error":{"type":"EntitlementError","message":"OpenCode Go subscription required."}}"#,
            ProviderErrorKind::Permission,
        ),
        (429, r#"{"type":"error"}"#, ProviderErrorKind::RateLimited),
    ] {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("auth.json"),
            r#"{"opencode-go":{"key":"test-key"}}"#,
        )
        .unwrap();
        let url = crate::providers::test_http::serve_once(status, &[], body);
        let provider = OpenCodeProvider::with_dependencies(
            OpenCodePaths::for_data_directory(directory.path().to_path_buf()),
            test_client(&url),
            now(),
        );
        assert_eq!(provider.refresh().unwrap_err().kind(), kind);
    }
}
