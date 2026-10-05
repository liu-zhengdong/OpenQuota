mod client;

mod mapper;
mod paths;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::models::{
    MetricDefinition, MetricSection, ProviderDefinition, ProviderErrorKind, ProviderLink,
    ProviderSnapshot,
};

use self::{client::OpenCodeClient, mapper::map_go_usage, paths::OpenCodePaths};

use super::{ProviderError, UsageProvider};

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

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenCodeError {
    #[error("OpenCode was not detected. Sign in to OpenCode Go.")]
    NotDetected,
    #[error("OpenCode login data could not be read. Sign in to OpenCode Go again.")]
    CredentialsUnreadable,
    #[error("OpenCode Go login data is invalid or expired. Sign in to OpenCode Go again.")]
    InvalidAuth,
    #[error("OpenCode Go subscription required.")]
    GoSubscriptionRequired,
    #[error("Could not reach OpenCode Go. Check your internet connection.")]
    ConnectionFailed,
    #[error("OpenCode Go returned an invalid usage response.")]
    InvalidResponse,
    #[error("OpenCode Go usage request failed (HTTP {0}).")]
    RequestFailed(u16),
}

impl From<OpenCodeError> for ProviderError {
    fn from(error: OpenCodeError) -> Self {
        let kind = match error {
            OpenCodeError::NotDetected | OpenCodeError::InvalidAuth => {
                ProviderErrorKind::Authentication
            }
            OpenCodeError::GoSubscriptionRequired => ProviderErrorKind::Permission,
            OpenCodeError::CredentialsUnreadable => ProviderErrorKind::CredentialStorage,
            OpenCodeError::ConnectionFailed => ProviderErrorKind::Network,
            OpenCodeError::RequestFailed(429) => ProviderErrorKind::RateLimited,
            OpenCodeError::RequestFailed(500..=599) => ProviderErrorKind::Network,
            OpenCodeError::InvalidResponse | OpenCodeError::RequestFailed(_) => {
                ProviderErrorKind::InvalidResponse
            }
        };
        ProviderError::new(kind, error.to_string())
    }
}

pub struct OpenCodeProvider {
    paths: OpenCodePaths,
    client: Result<OpenCodeClient, OpenCodeError>,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl OpenCodeProvider {
    fn refresh_snapshot(&self) -> Result<ProviderSnapshot, OpenCodeError> {
        let key = self.paths.go_api_key()?.ok_or(OpenCodeError::NotDetected)?;
        let quotas = map_go_usage(
            self.client
                .as_ref()
                .map_err(|error| *error)?
                .fetch_go_usage(&key)?,
        )?;
        Ok(snapshot(
            Some("Go".into()),
            quotas,
            Vec::new(),
            (self.now)(),
        ))
    }

    pub fn new() -> Self {
        let paths = OpenCodePaths::new();
        Self {
            paths,
            client: OpenCodeClient::new(),
            now: Arc::new(Utc::now),
        }
    }

    #[cfg(test)]
    fn with_dependencies(paths: OpenCodePaths, client: OpenCodeClient, now: DateTime<Utc>) -> Self {
        Self {
            paths,
            client: Ok(client),
            now: Arc::new(move || now),
        }
    }
}

impl UsageProvider for OpenCodeProvider {
    fn definition(&self) -> ProviderDefinition {
        definition()
    }

    fn has_local_credentials(&self) -> bool {
        match self.paths.go_api_key() {
            Ok(Some(_)) | Err(OpenCodeError::CredentialsUnreadable) => true,
            Ok(None) | Err(_) => false,
        }
    }

    fn refresh(&self) -> Result<ProviderSnapshot, ProviderError> {
        self.refresh_snapshot().map_err(ProviderError::from)
    }
}

fn snapshot(
    plan: Option<String>,
    quotas: Vec<crate::models::QuotaWindow>,
    warnings: Vec<crate::models::ProviderMessage>,
    refreshed_at: DateTime<Utc>,
) -> ProviderSnapshot {
    ProviderSnapshot {
        provider_id: "opencode".into(),
        plan,
        quotas,
        value_metrics: Vec::new(),
        status_metrics: Vec::new(),
        notices: Vec::new(),

        warnings,
        refreshed_at,
        remembered: false,
        account_identity: None,
        shared_scope: None,
    }
}

#[cfg(test)]
mod tests;
