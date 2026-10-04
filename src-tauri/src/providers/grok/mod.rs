use crate::models::{MetricDefinition, MetricSection, ProviderDefinition, ProviderLink};

pub(crate) fn definition() -> ProviderDefinition {
    ProviderDefinition {
        id: "grok".into(),
        display_name: "Grok".into(),
        short_name: "G".into(),
        fallback_enabled: false,
        scoped_quota_prefix: None,
        links: vec![ProviderLink::new("Usage", "https://grok.com/?_s=usage")],
        metrics: vec![
            MetricDefinition::quota(
                "grok.weekly",
                "Weekly",
                "weekly",
                false,
                true,
                MetricSection::AlwaysVisible,
                false,
                "W",
            ),
            MetricDefinition::status(
                "grok.payAsYouGo",
                "Extra Usage",
                "payAsYouGo",
                true,
                MetricSection::OnDemand,
                false,
                "E",
            ),
        ],
    }
}
