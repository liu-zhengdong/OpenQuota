use super::cache::cloud_facts;
use crate::models::ProviderSnapshot;

#[test]
fn cached_local_capacity_and_estimates_disappear_but_source_zero_and_success_time_survive() {
    let original: ProviderSnapshot = serde_json::from_value(serde_json::json!({
        "providerId":"commandcode", "plan":"SYNTHETIC",
        "quotas":[
            {"id":"credits","label":"Credits","usedPercent":90,"usedValue":9,"limitValue":10,"resetsAt":null,"periodSeconds":0},
            {"id":"weekly","label":"Weekly","usedPercent":12.345678,"usedValue":2,"limitValue":16.2,"resetsAt":null,"periodSeconds":604800}
        ],
        "valueMetrics":[{"id":"freeCredits","label":"Free","values":[{"number":0,"kind":"dollars","estimated":false}],"expiriesAt":[]}],
        "refreshedAt":"2026-10-03T00:00:00.123456Z"
    })).unwrap();
    let filtered = cloud_facts(original.clone());
    assert_eq!(filtered.quotas.len(), 1);
    assert_eq!(filtered.quotas[0], original.quotas[1]);
    assert_eq!(filtered.value_metrics, original.value_metrics);
    assert_eq!(filtered.refreshed_at, original.refreshed_at);
    let estimated: ProviderSnapshot = serde_json::from_value(serde_json::json!({
        "providerId":"codex","plan":null,"quotas":[],
        "valueMetrics":[{"id":"credits","label":"Credits","values":[
            {"number":0.4,"kind":"dollars","estimated":true},
            {"number":10.123456,"kind":"count","estimated":false}
        ],"expiriesAt":[]}],
        "refreshedAt":"2026-10-03T00:00:00.123456Z"
    }))
    .unwrap();
    let filtered = cloud_facts(estimated);
    assert_eq!(filtered.value_metrics[0].values.len(), 1);
    assert_eq!(filtered.value_metrics[0].values[0].number, 10.123456);
}
