mod auth;
mod client;
mod mapper;
#[cfg(test)]
mod tests;
use self::{auth::CommandCodeAuth, client::CommandCodeClient};
use super::{ProviderError, UsageProvider};
use crate::models::{
    MetricDefinition, MetricSection, ProviderDefinition, ProviderErrorKind, ProviderSnapshot,
};

pub(crate) fn definition() -> ProviderDefinition {
    use MetricSection::{AlwaysVisible, OnDemand};
    let mut metrics = vec![];
    for (id, label, section) in [
        ("remaining", "Total remaining", AlwaysVisible),
        ("freeCredits", "Free credits", AlwaysVisible),
        ("monthlyCredits", "Monthly remaining", OnDemand),
        ("purchasedCredits", "Purchased credits", OnDemand),
    ] {
        metrics.push(MetricDefinition::value(
            &format!("commandcode.{id}"),
            label,
            id,
            true,
            section,
            id == "remaining",
            "C",
            None,
        ));
    }
    for (id, label, session) in [("fiveHour", "5-hour", true), ("weekly", "Weekly", false)] {
        metrics.push(MetricDefinition::quota(
            &format!("commandcode.{id}"),
            label,
            id,
            session,
            true,
            AlwaysVisible,
            false,
            "W",
        ));
    }
    for metric in &mut metrics {
        metric.hide_when_missing = matches!(
            metric.id.as_str(),
            "commandcode.fiveHour" | "commandcode.weekly"
        );
    }
    metrics.push(MetricDefinition::status(
        "commandcode.daysLeft",
        "Days to renewal",
        "daysLeft",
        true,
        AlwaysVisible,
        false,
        "D",
    ));
    ProviderDefinition {
        id: "commandcode".into(),
        display_name: "Command Code".into(),
        short_name: "CC".into(),
        fallback_enabled: false,
        scoped_quota_prefix: None,
        links: vec![],
        metrics,
    }
}
#[derive(Debug, thiserror::Error)]
pub(super) enum CommandCodeError {
    #[error("Sign in with `command-code auth login` or set COMMAND_CODE_API_KEY.")]
    MissingKey,
    #[error("Command Code credentials could not be read. Sign in with `command-code auth login`.")]
    CredentialStorage,
    #[error("Could not reach Command Code. Check your internet connection.")]
    ConnectionFailed,
    #[error("Command Code usage data is temporarily unavailable.")]
    InvalidResponse,
    #[error("Command Code request failed (HTTP {0}). Sign in again if the session expired.")]
    RequestFailed(u16),
}
impl From<CommandCodeError> for ProviderError {
    fn from(error: CommandCodeError) -> Self {
        let kind = match error {
            CommandCodeError::MissingKey | CommandCodeError::RequestFailed(401 | 403) => {
                ProviderErrorKind::Authentication
            }
            CommandCodeError::RequestFailed(429 | 529) => ProviderErrorKind::RateLimited,
            CommandCodeError::ConnectionFailed => ProviderErrorKind::Network,
            CommandCodeError::CredentialStorage => ProviderErrorKind::CredentialStorage,
            _ => ProviderErrorKind::InvalidResponse,
        };
        ProviderError::new(kind, error.to_string())
    }
}
pub struct CommandCodeProvider {
    auth: CommandCodeAuth,
    client: CommandCodeClient,
}
impl CommandCodeProvider {
    pub fn new() -> Result<Self, ProviderError> {
        Ok(Self {
            auth: CommandCodeAuth::new(),
            client: CommandCodeClient::new()?,
        })
    }
}
impl UsageProvider for CommandCodeProvider {
    fn definition(&self) -> ProviderDefinition {
        definition()
    }
    fn has_local_credentials(&self) -> bool {
        self.auth.load().is_ok_and(|key| key.is_some())
    }
    fn refresh(&self) -> Result<ProviderSnapshot, ProviderError> {
        let key = self.auth.load()?.ok_or(CommandCodeError::MissingKey)?;
        Ok(mapper::map_usage(
            &self.client.fetch(key.as_str())?,
            chrono::Utc::now(),
        )?)
    }
}
