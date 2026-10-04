use super::*;

fn report() -> String {
    serde_json::json!({"object":"list", "data":[
        {"provider":"codex", "kind":"subscription", "plan":"prolite", "windows":[{"name":"5 hours","used":1.25,"remaining":98.75,"resetsAt":"2026-10-05T12:00:00Z"},{"name":"7 days","used":20,"remaining":80}]},
        {"provider":"grok-plugin", "kind":"subscription", "windows":[{"name":"7 days","used":99,"remaining":1},{"name":"On-demand","used":2,"remaining":98}]},
        {"provider":"kimi-code-cn", "kind":"plan", "windows":[{"name":"5 hours","used":100,"remaining":0},{"name":"7 days","used":21.125,"remaining":78.875}]},
        {"provider":"zcode", "kind":"subscription", "windows":[{"name":"5 hours","used":2,"remaining":98},{"name":"7 days","used":14,"remaining":86},{"name":"MCP · Month","used":3,"remaining":97}]},
        {"provider":"opencode-go", "kind":"plan", "windows":[{"name":"5 hours","used":0,"remaining":100},{"name":"7 days","used":100,"remaining":0},{"name":"Month","used":63,"remaining":37}]}
    ]}).to_string()
}

#[test]
fn isolated_http_report_preserves_every_window_and_catalog() {
    let body = report();
    let expected: Report = serde_json::from_str(&body).unwrap();
    let url = providers::test_http::serve_once(200, &[], &body);
    let runtimes = runtimes_at(url.strip_prefix("http://").unwrap()).unwrap();
    let registry = providers::ProviderRegistry::new(runtimes.clone()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let storage =
        Arc::new(crate::storage::Storage::open(&directory.path().join("openquota.db")).unwrap());
    let service = Arc::new(crate::service::ProviderService::new(
        Arc::new(providers::ProviderRegistry::new(runtimes.clone()).unwrap()),
        storage,
    ));
    for runtime in runtimes {
        let id = runtime.definition().id;
        assert!(runtime.has_local_credentials());
        let state = tauri::async_runtime::block_on(service.refresh(&id, true));
        assert!(state.error.is_none(), "{:?}", state.error);
        let snapshot = state.snapshot.unwrap();
        let original = mapper::select(&expected.data, &id).unwrap();
        assert_eq!(snapshot.quotas.len(), original.windows.len());
        assert!(!runtime.supports_api_key_configuration());
        for (actual, window) in snapshot.quotas.iter().zip(&original.windows) {
            assert_eq!(actual.used_percent, window.used);
            assert!(((100.0 - actual.used_percent).max(0.0) - window.remaining).abs() < 1e-7);
            assert_eq!(actual.resets_at, window.resets_at);
            assert_eq!(actual.label, window.name);
            let metric_id = format!("{id}.{}", actual.id);
            assert!(registry.metric(&metric_id).is_some(), "{metric_id}");
            println!(
                "{id}/{} used={} remaining={} reset={:?}: identical",
                window.name, actual.used_percent, window.remaining, actual.resets_at
            );
        }
        assert!(matches!(
            runtime.cache_identity(),
            CacheIdentity::Unresolved
        ));
    }
}

#[test]
fn rejects_broken_reports_and_ambiguous_accounts() {
    for body in [
        "{}",
        "{\"object\":\"wrong\",\"data\":[]}",
        "{\"object\":\"list\",\"data\":[{\"provider\":\"codex\"}]}",
    ] {
        let url = providers::test_http::serve_once(200, &[], body);
        let source = Source::new(url.strip_prefix("http://").unwrap()).unwrap();
        assert!(source.read().is_err());
    }
    let parsed: Report = serde_json::from_str(&report()).unwrap();
    let mut duplicate = parsed.data.clone();
    duplicate.push(parsed.data[0].clone());
    assert!(mapper::select(&duplicate, "codex").is_err());
    assert!(mapper::select(&parsed.data, "cursor").is_err());
    assert!(mapper::select(&parsed.data, "claude").is_err());
    let mut broken = parsed.data[0].clone();
    broken.error = "secret upstream response".into();
    let error = mapper::select(&[broken], "codex").err().unwrap();
    assert!(!error.to_string().contains("secret"));
    for mutate in 0..5 {
        let mut broken = parsed.data[0].clone();
        match mutate {
            0 => broken.windows[0].used = f64::NAN,
            1 => broken.windows[0].remaining = -1.0,
            2 => broken.windows.push(broken.windows[0].clone()),
            3 => broken.windows[0].unlimited = true,
            _ => broken.windows[0].remaining = 10.0,
        }
        assert!(mapper::snapshot(&broken, "codex").is_err());
    }
    for status in [403, 429, 500] {
        let url = providers::test_http::serve_once(status, &[], "secret response");
        let source = Source::new(url.strip_prefix("http://").unwrap()).unwrap();
        let error = source.read().err().unwrap();
        assert!(!error.to_string().contains("secret"));
    }
    assert!(Source::new("example.com:3425").is_err());
}

#[test]
fn dated_magpie_readings_stay_remembered() {
    let mut parsed: Report = serde_json::from_str(&report()).unwrap();
    parsed.data[0].as_of = Some("2026-10-01T00:00:00Z".parse().unwrap());
    let snapshot = mapper::snapshot(&parsed.data[0], "codex").unwrap();
    assert!(snapshot.remembered);
    assert_eq!(Some(snapshot.refreshed_at), parsed.data[0].as_of);
}

pub(super) fn sample_snapshots() -> Vec<ProviderSnapshot> {
    let report: Report = serde_json::from_str(&report()).unwrap();
    ["codex", "grok", "kimi", "zai", "opencode"]
        .into_iter()
        .map(|id| mapper::snapshot(mapper::select(&report.data, id).unwrap(), id).unwrap())
        .collect()
}
