use crate::models::{MetricDefinition, MetricSection, ProviderDefinition, ProviderLink};

pub(crate) fn definition() -> ProviderDefinition {
    ProviderDefinition {
        id: "opencode".into(),
        display_name: "OpenCode".into(),
        short_name: "OC".into(),
        fallback_enabled: false,
        scoped_quota_prefix: None,
        links: vec![ProviderLink::new("Dashboard", "https://opencode.ai/auth")],
        metrics: vec![
            MetricDefinition::quota(
                "opencode.session",
                "Session",
                "session",
                true,
                true,
                MetricSection::AlwaysVisible,
                false,
                "S",
            ),
            MetricDefinition::quota(
                "opencode.weekly",
                "Weekly",
                "weekly",
                false,
                true,
                MetricSection::AlwaysVisible,
                false,
                "W",
            ),
            MetricDefinition::quota(
                "opencode.monthly",
                "Monthly",
                "monthly",
                false,
                true,
                MetricSection::AlwaysVisible,
                false,
                "M",
            ),
        ],
    }
}
