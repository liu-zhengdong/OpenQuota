//! Remove local estimates from cached provider facts until the next successful refresh replaces them.
//! This does not change success timestamps, identity, or source balances.
use crate::models::ProviderSnapshot;

pub fn cloud_facts(mut snapshot: ProviderSnapshot) -> ProviderSnapshot {
    // Releases through PR16 always derived this capacity from a local plan table or spent+remaining.
    if snapshot.provider_id == "commandcode" {
        snapshot.quotas.retain(|window| window.id != "credits");
    }
    snapshot.quotas.retain(|window| !window.estimated);
    for metric in &mut snapshot.value_metrics {
        metric.values.retain(|value| !value.estimated);
    }
    snapshot
        .value_metrics
        .retain(|metric| !metric.values.is_empty());
    snapshot
}
