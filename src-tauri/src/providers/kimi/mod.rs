use crate::models::{MetricDefinition, MetricSection, ProviderDefinition, ProviderLink};

pub(crate) fn definition() -> ProviderDefinition {
    ProviderDefinition {
        id: "kimi".into(),
        display_name: "Kimi".into(),
        short_name: "K".into(),
        fallback_enabled: false,
        scoped_quota_prefix: None,
        links: vec![
            ProviderLink::new("Dashboard", "https://www.kimi.com/code/console"),
            ProviderLink::new("API Keys", "https://www.kimi.com/code/console"),
        ],
        metrics: vec![
            MetricDefinition::quota(
                "kimi.session",
                "Session",
                "session",
                false,
                true,
                MetricSection::AlwaysVisible,
                true,
                "S",
            ),
            MetricDefinition::quota(
                "kimi.weekly",
                "Weekly",
                "weekly",
                false,
                true,
                MetricSection::AlwaysVisible,
                true,
                "W",
            ),
        ],
    }
}
