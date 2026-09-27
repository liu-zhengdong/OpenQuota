mod accounts;
pub mod auth;
mod client;
mod mapper;

/// Catalog identifier the interface resolves against `notices.claude.rateLimited`.
const RATE_LIMIT_NOTICE_ID: &str = "claude.rateLimited";

use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use reqwest::StatusCode;
use thiserror::Error;

use crate::{
    models::{
        MetricDefinition, MetricSection, ProviderDefinition, ProviderLink, ProviderMessage,
        ProviderNotice, ProviderNoticeTone, ProviderSnapshot,
    },
    providers::CacheIdentity,
    storage::Storage,
};

pub(crate) fn definition() -> ProviderDefinition {
    definition_for("claude", "Claude", true)
}

fn definition_for(id: &str, display_name: &str, fallback_enabled: bool) -> ProviderDefinition {
    let mut definition = ProviderDefinition {
        id: "claude".into(),
        display_name: display_name.into(),
        short_name: "Cl".into(),
        fallback_enabled,
        scoped_quota_prefix: Some("scoped-".into()),
        links: vec![
            ProviderLink::new("Status", "https://status.anthropic.com/"),
            ProviderLink::new("Dashboard", "https://claude.ai/settings/usage"),
        ],
        metrics: vec![
            MetricDefinition::quota(
                "claude.session",
                "Session",
                "session",
                true,
                true,
                MetricSection::AlwaysVisible,
                true,
                "S",
            ),
            MetricDefinition::quota(
                "claude.weekly",
                "Weekly",
                "weekly",
                false,
                true,
                MetricSection::AlwaysVisible,
                true,
                "W",
            ),
            MetricDefinition::quota(
                "claude.sonnet",
                "Sonnet",
                "sonnet",
                false,
                false,
                MetricSection::OnDemand,
                false,
                "Sn",
            ),
            scoped_metric("claude", "fable", "Fable"),
            MetricDefinition::quota_or_value(
                "claude.extra",
                "Extra Usage",
                "extra",
                true,
                MetricSection::AlwaysVisible,
                false,
                "E",
            ),
        ],
    };
    if id != "claude" {
        definition.id = id.into();
        for metric in &mut definition.metrics {
            if let Some(suffix) = metric.id.strip_prefix("claude.") {
                metric.id = format!("{id}.{suffix}");
            }
            metric.default_pinned = false;
        }
    }
    definition
}

// Model windows are supplied by the API, while the legacy Fable row keeps its saved layout.
pub(crate) fn scoped_metric(provider_id: &str, id: &str, label: &str) -> MetricDefinition {
    let legacy = id == "fable";
    MetricDefinition::quota(
        &format!("{provider_id}.{id}"),
        label,
        id,
        false,
        !legacy,
        if legacy {
            MetricSection::OnDemand
        } else {
            MetricSection::AlwaysVisible
        },
        false,
        if legacy { "F" } else { label },
    )
}

use self::{
    auth::{
        load_candidates, oauth_config, ClaudeCredential, ClaudeCredentialGeneration,
        ClaudeCredentialScope,
    },
    client::ClaudeClient,
    mapper::map_usage,
};

#[derive(Debug, Error)]
pub enum ClaudeError {
    #[error("Not logged in. Run `claude` to authenticate.")]
    NotLoggedIn,
    #[error(
        "Claude Desktop login found, but its macOS-only encrypted session cannot be reused safely. Run `claude` in a terminal and sign in once."
    )]
    DesktopAppOnly,
    #[error("Your Claude session expired. Run `claude` to sign in again.")]
    SessionExpired,
    #[error("Your Claude token expired. Run `claude` to sign in again.")]
    TokenExpired,
    #[error("Claude OAuth settings contain an invalid URL.")]
    InvalidOAuthUrl,
    #[error("Refreshed Claude credentials could not be saved.")]
    AuthWrite,
    #[error("Claude login changed during refresh. Refresh again.")]
    CredentialsChanged,
    #[error("The Claude account changed while OpenQuota was running. Restart OpenQuota to reconnect it safely.")]
    AccountChanged,
    #[error("Claude usage request failed (HTTP {0}).")]
    RequestFailed(u16),
    #[error("Claude returned an invalid usage response.")]
    InvalidResponse,
    #[error("Could not connect to Claude. Check your internet connection.")]
    ConnectionFailed,
    #[error("Claude account settings could not be loaded.")]
    AccountStore(#[from] crate::storage::StorageError),
}

pub struct ClaudeProvider {
    definition: ProviderDefinition,
    credential_scope: ClaudeCredentialScope,
    account_identity: Option<String>,
    client: ClaudeClient,
    cached_credential_fingerprint: Mutex<Option<[u8; 32]>>,
    last_good: Mutex<Option<ProviderSnapshot>>,
    rate_limited_until: Mutex<Option<chrono::DateTime<Utc>>>,
    /// Lets a cold start inside a rate limit window show the numbers the last run read
    /// instead of an empty card. `None` only in tests that never build a `Storage`.
    storage: Option<Arc<Storage>>,
}

struct ClaudeRuntimeConfig {
    definition: ProviderDefinition,
    credential_scope: ClaudeCredentialScope,
    account_identity: Option<String>,
}

pub(crate) fn runtimes(
    storage: Arc<Storage>,
) -> Result<Vec<Arc<dyn crate::providers::UsageProvider>>, ClaudeError> {
    let discovery = accounts::discover(&storage)?;
    let client = ClaudeClient::new()?;
    Ok(runtime_configs(discovery)
        .into_iter()
        .map(|config| {
            Arc::new(ClaudeProvider::new_scoped(
                config,
                client.clone(),
                Some(storage.clone()),
            )) as Arc<dyn crate::providers::UsageProvider>
        })
        .collect())
}

fn runtime_configs(discovery: accounts::ClaudeAccountDiscovery) -> Vec<ClaudeRuntimeConfig> {
    let mut configs = Vec::new();
    // A bare-ID account that moved to a config dir is still the `claude` card, so it replaces the
    // empty default-login placeholder instead of colliding with a second runtime of the same ID.
    let has_bare_scoped_account = discovery
        .accounts
        .iter()
        .any(|account| account.id == "claude");
    if let Some(account) = discovery.default_account {
        configs.push(ClaudeRuntimeConfig {
            definition: definition_for(&account.id, &account.display_name, true),
            credential_scope: ClaudeCredentialScope::Standard,
            account_identity: Some(account.identity),
        });
    } else if !has_bare_scoped_account {
        configs.push(ClaudeRuntimeConfig {
            definition: definition(),
            credential_scope: ClaudeCredentialScope::Standard,
            account_identity: None,
        });
    }
    for account in discovery.accounts {
        configs.push(ClaudeRuntimeConfig {
            definition: definition_for(&account.id, &account.display_name, false),
            credential_scope: account.credential_scope,
            account_identity: Some(account.identity),
        });
    }
    configs
}

impl ClaudeProvider {
    #[cfg(test)]
    fn new() -> Self {
        Self::new_scoped(
            ClaudeRuntimeConfig {
                definition: definition(),
                credential_scope: ClaudeCredentialScope::Standard,
                account_identity: None,
            },
            ClaudeClient::new().expect("test client"),
            None,
        )
    }

    fn new_scoped(
        config: ClaudeRuntimeConfig,
        client: ClaudeClient,
        storage: Option<Arc<Storage>>,
    ) -> Self {
        let provider = Self {
            definition: config.definition,
            credential_scope: config.credential_scope,
            account_identity: config.account_identity,
            client,
            cached_credential_fingerprint: Mutex::new(None),
            last_good: Mutex::new(None),
            rate_limited_until: Mutex::new(None),
            storage,
        };
        provider.restore_last_good();
        provider
    }

    /// Seeds the in-memory fallback from the last snapshot this account left on disk.
    ///
    /// The service already persists every successful snapshot, so a restart during a
    /// rate limit window can reuse it. Provider-authored notices and warnings are
    /// dropped: the window this snapshot is replayed into decides those again.
    fn restore_last_good(&self) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        // Mirrors `cache_identity` so the row found here is the row a refresh would write.
        let identity = match self.account_identity.as_deref() {
            Some(identity) => CacheIdentity::Resolved(identity),
            None => CacheIdentity::Unresolved,
        };
        match storage.load_snapshot_for_identity(self.provider_id(), identity) {
            Ok(Some(snapshot)) => {
                crate::app_debug!(
                    "cache",
                    "reusing the last successful Claude limits from disk for {}",
                    self.provider_id()
                );
                if let Ok(mut last) = self.last_good.lock() {
                    *last = Some(remembered_snapshot(snapshot));
                }
            }
            Ok(None) => {}
            Err(error) => crate::app_warn!(
                "cache",
                "last successful Claude limits could not be read for {}: {error}",
                self.provider_id()
            ),
        }
    }

    fn provider_id(&self) -> &str {
        &self.definition.id
    }

    fn refresh_inner(&self) -> Result<ProviderSnapshot, ClaudeError> {
        let config = oauth_config()?;
        self.refresh_inner_with_config(&config)
    }

    fn refresh_inner_with_config(
        &self,
        config: &auth::ClaudeOAuthConfig,
    ) -> Result<ProviderSnapshot, ClaudeError> {
        let mut credential_reloads_remaining = 1;
        loop {
            match self.refresh_inner_once(config) {
                Err(ClaudeError::CredentialsChanged) if credential_reloads_remaining > 0 => {
                    credential_reloads_remaining -= 1;
                    crate::app_info!(
                        "auth:claude",
                        "credential source changed during refresh; reloading current login"
                    );
                }
                Ok(snapshot) => {
                    self.ensure_account_identity_current()?;
                    return Ok(snapshot);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn refresh_inner_once(
        &self,
        config: &auth::ClaudeOAuthConfig,
    ) -> Result<ProviderSnapshot, ClaudeError> {
        self.ensure_account_identity_current()?;
        let candidates = load_candidates(&self.credential_scope);
        if candidates.is_empty() {
            crate::app_info!("auth:claude", "no reusable CLI credentials found");
            return Err(
                if matches!(self.credential_scope, ClaudeCredentialScope::Standard)
                    && auth::has_desktop_app_data()
                {
                    ClaudeError::DesktopAppOnly
                } else {
                    ClaudeError::NotLoggedIn
                },
            );
        }
        crate::app_debug!(
            "auth:claude",
            "credential candidates loaded ({})",
            candidates.len()
        );
        let now = Utc::now();
        let mut credential_generation = ClaudeCredentialGeneration::from_candidates(&candidates);
        let mut last_auth_error = None;
        for mut credential in candidates {
            match self.refresh_candidate(&mut credential, config, now, &mut credential_generation) {
                Ok(snapshot) => return Ok(snapshot),
                Err(error @ (ClaudeError::SessionExpired | ClaudeError::TokenExpired)) => {
                    last_auth_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_auth_error.unwrap_or(ClaudeError::NotLoggedIn))
    }

    fn ensure_account_identity_current(&self) -> Result<(), ClaudeError> {
        let Some(expected) = self.account_identity.as_deref() else {
            return Ok(());
        };
        (accounts::identity_for_scope(&self.credential_scope).as_deref() == Some(expected))
            .then_some(())
            .ok_or(ClaudeError::AccountChanged)
    }

    fn refresh_candidate(
        &self,
        credential: &mut ClaudeCredential,
        config: &auth::ClaudeOAuthConfig,
        now: chrono::DateTime<Utc>,
        credential_generation: &mut ClaudeCredentialGeneration,
    ) -> Result<ProviderSnapshot, ClaudeError> {
        let mut warnings = Vec::new();

        if credential.inference_only {
            return Ok(ProviderSnapshot {
                provider_id: self.provider_id().into(),
                plan: plan_name(credential),
                quotas: Vec::new(),
                value_metrics: Vec::new(),
                status_metrics: Vec::new(),
                notices: Vec::new(),

                warnings,
                refreshed_at: now,
                remembered: false,
            });
        }
        if !credential.has_profile_scope() {
            warnings.push(ProviderMessage::new(
                "claude.reLoginRequired",
                "Re-login for live usage. Run `claude` and sign in again to restore subscription limits.",
            ));
            return Ok(ProviderSnapshot {
                provider_id: self.provider_id().into(),
                plan: plan_name(credential),
                quotas: Vec::new(),
                value_metrics: Vec::new(),
                status_metrics: Vec::new(),
                notices: Vec::new(),

                warnings,
                refreshed_at: now,
                remembered: false,
            });
        }
        self.activate_live_usage_cache(credential.fingerprint());
        if credential.needs_refresh(now.timestamp_millis()) {
            let previous_fingerprint = credential.fingerprint();
            refresh_credential(
                &self.client,
                credential,
                config,
                now,
                &mut warnings,
                credential_generation,
                &self.credential_scope,
            )?;
            self.replace_live_usage_fingerprint(previous_fingerprint, credential.fingerprint());
        }

        let cooldown_until = self
            .rate_limited_until
            .lock()
            .ok()
            .and_then(|value| *value)
            .filter(|until| now < *until);
        if let Some(until) = cooldown_until {
            let retry = until.signed_duration_since(now).num_seconds().max(0) as u64;
            return Ok(self.rate_limited_snapshot(plan_name(credential), retry, warnings));
        }

        let token = credential.access_token().ok_or(ClaudeError::NotLoggedIn)?;
        let (mut status, mut body, mut retry_after) = self.client.fetch_usage(token, config)?;
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            let previous_fingerprint = credential.fingerprint();
            refresh_credential(
                &self.client,
                credential,
                config,
                now,
                &mut warnings,
                credential_generation,
                &self.credential_scope,
            )?;
            self.replace_live_usage_fingerprint(previous_fingerprint, credential.fingerprint());
            let token = credential.access_token().ok_or(ClaudeError::TokenExpired)?;
            (status, body, retry_after) = self.client.fetch_usage(token, config)?;
        }
        if auth::credential_generation(&self.credential_scope) != *credential_generation {
            return Err(ClaudeError::CredentialsChanged);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry = retry_after.unwrap_or(5 * 60);
            if let Ok(mut until) = self.rate_limited_until.lock() {
                *until = Some(now + Duration::seconds(retry as i64));
            }
            return Ok(self.rate_limited_snapshot(plan_name(credential), retry, warnings));
        }
        self.build_snapshot(status, &body, credential, warnings, now)
    }

    /// The snapshot shown while the usage endpoint is refusing requests.
    ///
    /// Replays the last successful limits when they are known, keeping their original
    /// `refreshed_at` so the interface can say how old the numbers are. Either way the
    /// result is marked `remembered`: nothing was read from the provider, so it must never
    /// replace the limits already on disk.
    fn rate_limited_snapshot(
        &self,
        plan: Option<String>,
        retry: u64,
        warnings: Vec<ProviderMessage>,
    ) -> ProviderSnapshot {
        let Some(mut snapshot) = self.last_good.lock().ok().and_then(|value| value.clone()) else {
            let mut warnings = warnings;
            warnings.push(
                ProviderMessage::new(
                    "claude.rateLimitedRetry",
                    format!(
                        "Claude live usage is rate limited; retrying in about {}.",
                        retry_minutes(retry)
                    ),
                )
                .with_param("retrySeconds", retry.to_string()),
            );
            return ProviderSnapshot {
                provider_id: self.provider_id().into(),
                plan,
                quotas: Vec::new(),
                value_metrics: Vec::new(),
                status_metrics: Vec::new(),
                notices: vec![rate_limit_notice(retry, false, Utc::now())],
                warnings,
                refreshed_at: Utc::now(),
                remembered: true,
            };
        };
        snapshot.remembered = true;
        // Whatever the caller collected before the rate limit still applies to this card.
        snapshot.warnings.splice(0..0, warnings);
        snapshot.warnings.push(ProviderMessage::new(
            "claude.rateLimitedStale",
            "Claude live usage is rate limited; showing the last successful limits.",
        ));
        snapshot.notices = vec![rate_limit_notice(retry, true, snapshot.refreshed_at)];
        snapshot
    }

    fn build_snapshot(
        &self,
        status: StatusCode,
        body: &serde_json::Value,
        credential: &ClaudeCredential,
        warnings: Vec<ProviderMessage>,
        now: chrono::DateTime<Utc>,
    ) -> Result<ProviderSnapshot, ClaudeError> {
        let mapped = map_usage(status, body, &credential.oauth)?;
        let snapshot = ProviderSnapshot {
            provider_id: self.provider_id().into(),
            plan: mapped.plan,
            quotas: mapped.quotas,
            value_metrics: mapped.value_metrics,
            status_metrics: Vec::new(),
            notices: Vec::new(),

            warnings,
            refreshed_at: now,
            remembered: false,
        };
        if let Ok(mut last) = self.last_good.lock() {
            *last = Some(snapshot.clone());
        }
        if let Ok(mut until) = self.rate_limited_until.lock() {
            *until = None;
        }
        Ok(snapshot)
    }

    /// Points the live usage cache at `fingerprint`, dropping the remembered limits when the
    /// credential behind them changed.
    ///
    /// The first activation of a process only adopts: there is no earlier fingerprint to have
    /// swapped from, and `restore_last_good` already keyed what it loaded on the account
    /// identity resolved at startup.
    fn activate_live_usage_cache(&self, fingerprint: [u8; 32]) {
        let previous = self
            .cached_credential_fingerprint
            .lock()
            .ok()
            .and_then(|active| *active);
        if previous == Some(fingerprint) {
            return;
        }
        if let Ok(mut active) = self.cached_credential_fingerprint.lock() {
            *active = Some(fingerprint);
        }
        if previous.is_none() {
            return;
        }
        if let Ok(mut last) = self.last_good.lock() {
            *last = None;
        }
        if let Ok(mut until) = self.rate_limited_until.lock() {
            *until = None;
        }
    }

    fn replace_live_usage_fingerprint(&self, previous: [u8; 32], current: [u8; 32]) {
        if let Ok(mut active) = self.cached_credential_fingerprint.lock() {
            if active.as_ref() == Some(&previous) {
                *active = Some(current);
            }
        }
    }
}

/// Builds the `claude.rateLimited` notice.
///
/// `variant` names the catalog entry holding the message, so the interface picks the copy for
/// its own language. `remembered_at` is the moment the replayed limits were actually read, so
/// the panel can label the numbers instead of implying the retry window refreshed them.
fn rate_limit_notice(
    retry_seconds: u64,
    showing_remembered_limits: bool,
    remembered_at: chrono::DateTime<Utc>,
) -> ProviderNotice {
    let variant = match (showing_remembered_limits, retry_seconds) {
        (_, 0) => "ready",
        (true, _) => "rememberedRetry",
        (false, _) => "retry",
    };
    let mut notice = ProviderNotice::new(
        RATE_LIMIT_NOTICE_ID,
        "Live usage paused",
        if showing_remembered_limits {
            format!(
                "Showing the last successful limits · {}",
                retry_phrase(retry_seconds)
            )
        } else {
            retry_phrase(retry_seconds)
        },
        ProviderNoticeTone::Warning,
    )
    .with_param("variant", variant)
    .with_param("retrySeconds", retry_seconds.to_string());
    if showing_remembered_limits {
        notice = notice.with_param("rememberedAt", remembered_at.to_rfc3339());
    }
    notice
}

fn retry_phrase(retry_seconds: u64) -> String {
    if retry_seconds == 0 {
        return "Ready to retry".to_owned();
    }
    format!("Retrying in about {}", retry_minutes(retry_seconds))
}

/// Strips provider-authored presentation from a snapshot replayed as remembered data.
fn remembered_snapshot(mut snapshot: ProviderSnapshot) -> ProviderSnapshot {
    snapshot.notices = Vec::new();
    snapshot.warnings = Vec::new();
    snapshot.remembered = true;
    snapshot
}

fn retry_minutes(retry_seconds: u64) -> String {
    let minutes = retry_seconds.div_ceil(60);
    format!(
        "{minutes} {}",
        if minutes == 1 { "minute" } else { "minutes" }
    )
}

fn refresh_credential(
    client: &ClaudeClient,
    credential: &mut ClaudeCredential,
    config: &auth::ClaudeOAuthConfig,
    now: chrono::DateTime<Utc>,
    warnings: &mut Vec<ProviderMessage>,
    credential_generation: &mut ClaudeCredentialGeneration,
    credential_scope: &ClaudeCredentialScope,
) -> Result<(), ClaudeError> {
    let refresh_token = credential
        .oauth
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(ClaudeError::TokenExpired)?;
    let refreshed = client.refresh_token(refresh_token, config)?;
    match credential.update_and_save(
        refreshed.access_token,
        refreshed.refresh_token,
        refreshed.expires_in,
        now.timestamp_millis(),
        credential_generation,
        credential_scope,
    ) {
        Ok(true) => {
            *credential_generation = credential_generation
                .replacing(credential)
                .ok_or(ClaudeError::CredentialsChanged)?;
        }
        Ok(false) => {}
        Err(ClaudeError::CredentialsChanged) => return Err(ClaudeError::CredentialsChanged),
        Err(_) => {
            crate::app_error!(
                "auth:claude",
                "failed to persist rotated credentials; using them for this session only"
            );
            warnings.push(ProviderMessage::new(
                "claude.credentialNotSaved",
                "The refreshed Claude login is active for this session but could not be saved.",
            ));
        }
    }
    Ok(())
}

fn plan_name(credential: &ClaudeCredential) -> Option<String> {
    credential.oauth.subscription_type.as_ref().map(|value| {
        let mut chars = value.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    })
}

impl crate::providers::UsageProvider for ClaudeProvider {
    fn definition(&self) -> ProviderDefinition {
        self.definition.clone()
    }

    fn has_local_credentials(&self) -> bool {
        auth::has_local_credentials(&self.credential_scope)
    }

    fn cache_identity(&self) -> crate::providers::CacheIdentity<'_> {
        self.account_identity
            .as_deref()
            .map(crate::providers::CacheIdentity::Resolved)
            .unwrap_or(crate::providers::CacheIdentity::Unresolved)
    }

    fn supports_account_names(&self) -> bool {
        true
    }

    fn account_identity(&self) -> Option<&str> {
        self.account_identity.as_deref()
    }

    fn refresh(&self) -> Result<ProviderSnapshot, crate::providers::ProviderError> {
        self.refresh_inner().map_err(|error| {
            use crate::models::ProviderErrorKind as Kind;

            let kind = match error {
                ClaudeError::NotLoggedIn
                | ClaudeError::DesktopAppOnly
                | ClaudeError::SessionExpired
                | ClaudeError::TokenExpired
                | ClaudeError::CredentialsChanged
                | ClaudeError::AccountChanged => Kind::Authentication,
                ClaudeError::InvalidOAuthUrl | ClaudeError::InvalidResponse => {
                    Kind::InvalidResponse
                }
                ClaudeError::AuthWrite => Kind::CredentialStorage,
                ClaudeError::RequestFailed(429) => Kind::RateLimited,
                ClaudeError::RequestFailed(_) | ClaudeError::ConnectionFailed => Kind::Network,
                ClaudeError::AccountStore(_) => Kind::Internal,
            };
            crate::providers::ProviderError::from_display(kind, error)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        sync::{mpsc, Arc},
        thread,
        time::Duration as StdDuration,
    };

    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use crate::models::{
        ProviderMessage, ProviderNotice, ProviderNoticeTone, ProviderSnapshot, QuotaFormat,
        QuotaWindow,
    };
    use crate::storage::Storage;

    use super::{
        accounts::{self, ClaudeAccount, ClaudeAccountDiscovery},
        auth::{ClaudeCredentialScope, ClaudeOAuthConfig},
        client::ClaudeClient,
        definition, definition_for, rate_limit_notice, runtime_configs, ClaudeError,
        ClaudeProvider, ClaudeRuntimeConfig,
    };

    fn credential_json(access: &str, refresh: &str, plan: &str) -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{access}","refreshToken":"{refresh}","expiresAt":4102444800000,"subscriptionType":"{plan}","scopes":["user:profile"]}}}}"#
        )
    }

    fn write_http_response(stream: &mut impl Write, utilization: u8) {
        let body = format!(
            r#"{{"five_hour":{{"utilization":{utilization},"resets_at":"2099-01-01T00:00:00Z"}}}}"#
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }

    /// A snapshot shaped like a successful weekly read, optionally carrying the notice and
    /// warning a previous rate limit window would have persisted.
    fn remembered_limit_snapshot(
        used_percent: f64,
        refreshed_at: chrono::DateTime<Utc>,
        with_rate_limit_presentation: bool,
    ) -> ProviderSnapshot {
        ProviderSnapshot {
            provider_id: "claude".into(),
            plan: Some("Max".into()),
            quotas: vec![QuotaWindow {
                id: "claude.weekly".into(),
                label: "Weekly".into(),
                used_percent,
                resets_at: None,
                period_seconds: 604_800,
                format: QuotaFormat::Percent,
                used_value: None,
                limit_value: None,
                unit: None,
                estimated: false,
                source_note: None,
            }],
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: with_rate_limit_presentation
                .then(|| {
                    ProviderNotice::new(
                        "claude.rateLimited",
                        "Live usage paused",
                        "Ready to retry",
                        ProviderNoticeTone::Warning,
                    )
                })
                .into_iter()
                .collect(),
            warnings: with_rate_limit_presentation
                .then(|| ProviderMessage::new("claude.rateLimitedStale", "previous window copy"))
                .into_iter()
                .collect(),
            refreshed_at,
            remembered: false,
        }
    }

    #[test]
    fn account_definition_has_isolated_provider_and_metric_ids() {
        let definition = definition_for("claude@1234abcd", "Claude — Work", false);

        assert_eq!(definition.id, "claude@1234abcd");
        assert_eq!(definition.display_name, "Claude — Work");
        assert!(!definition.fallback_enabled);
        assert!(definition
            .metrics
            .iter()
            .all(|metric| metric.id.starts_with("claude@1234abcd.")));
        assert!(definition
            .metrics
            .iter()
            .all(|metric| !metric.default_pinned));
    }

    #[test]
    fn bare_account_in_a_config_dir_replaces_the_empty_default_placeholder() {
        let configs = runtime_configs(ClaudeAccountDiscovery {
            default_account: None,
            accounts: vec![ClaudeAccount {
                id: "claude".into(),
                display_name: "Claude".into(),
                label: Some("Personal".into()),
                identity: "identity-a".into(),
                credential_scope: ClaudeCredentialScope::ConfigDir {
                    path: "account-a".into(),
                    keychain_literal: "account-a".into(),
                },
            }],
        });

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].definition.id, "claude");
        assert!(matches!(
            &configs[0].credential_scope,
            ClaudeCredentialScope::ConfigDir { .. }
        ));
    }

    #[test]
    fn swapped_accounts_keep_their_ids_while_sources_change() {
        let configs = runtime_configs(ClaudeAccountDiscovery {
            default_account: Some(ClaudeAccount {
                id: "claude@1234abcd".into(),
                display_name: "Claude — Work".into(),
                label: Some("Work".into()),
                identity: "identity-b".into(),
                credential_scope: ClaudeCredentialScope::Standard,
            }),
            accounts: vec![ClaudeAccount {
                id: "claude".into(),
                display_name: "Claude".into(),
                label: Some("Personal".into()),
                identity: "identity-a".into(),
                credential_scope: ClaudeCredentialScope::ConfigDir {
                    path: "account-a".into(),
                    keychain_literal: "account-a".into(),
                },
            }],
        });

        assert_eq!(
            configs
                .iter()
                .map(|config| config.definition.id.as_str())
                .collect::<Vec<_>>(),
            ["claude@1234abcd", "claude"]
        );
        assert!(matches!(
            &configs[0].credential_scope,
            ClaudeCredentialScope::Standard
        ));
        assert!(matches!(
            &configs[1].credential_scope,
            ClaudeCredentialScope::ConfigDir { .. }
        ));
    }

    #[test]
    fn rate_limit_notice_distinguishes_empty_and_remembered_live_usage() {
        let read_at = Utc::now();

        let empty = rate_limit_notice(301, false, read_at);
        assert_eq!(empty.id, "claude.rateLimited");
        assert_eq!(empty.title, "Live usage paused");
        assert_eq!(empty.message, "Retrying in about 6 minutes");
        assert_eq!(empty.tone, ProviderNoticeTone::Warning);
        assert_eq!(
            empty.params.get("retrySeconds").map(String::as_str),
            Some("301")
        );
        assert_eq!(
            empty.params.get("variant").map(String::as_str),
            Some("retry")
        );
        assert!(!empty.params.contains_key("rememberedAt"));

        let ready = rate_limit_notice(0, false, read_at);
        assert_eq!(ready.message, "Ready to retry");
        assert_eq!(
            ready.params.get("variant").map(String::as_str),
            Some("ready")
        );

        let remembered = rate_limit_notice(60, true, read_at);
        assert_eq!(
            remembered.message,
            "Showing the last successful limits · Retrying in about 1 minute"
        );
        assert_eq!(
            remembered.params.get("variant").map(String::as_str),
            Some("rememberedRetry")
        );
        assert_eq!(
            remembered.params.get("rememberedAt").map(String::as_str),
            Some(read_at.to_rfc3339().as_str())
        );
    }

    #[test]
    fn rate_limited_snapshot_without_remembered_limits_carries_retry_only() {
        let provider = ClaudeProvider::new();

        let snapshot = provider.rate_limited_snapshot(Some("Max".into()), 300, Vec::new());

        assert!(snapshot.quotas.is_empty());
        assert!(snapshot.remembered);
        assert_eq!(snapshot.notices[0].id, "claude.rateLimited");
        assert_eq!(snapshot.notices[0].message, "Retrying in about 5 minutes");
        assert_eq!(snapshot.warnings[0].id, "claude.rateLimitedRetry");
        assert_eq!(
            snapshot.warnings[0]
                .params
                .get("retrySeconds")
                .map(String::as_str),
            Some("300")
        );
    }

    #[test]
    fn rate_limited_snapshot_replays_remembered_limits_with_their_read_time() {
        let provider = ClaudeProvider::new();
        let read_at = Utc::now() - Duration::minutes(42);
        *provider.last_good.lock().unwrap() = Some(remembered_limit_snapshot(31.0, read_at, false));

        let snapshot = provider.rate_limited_snapshot(None, 42, Vec::new());

        assert_eq!(snapshot.quotas[0].used_percent, 31.0);
        assert!(snapshot.remembered);
        // The numbers are only as fresh as the read they came from, so the card must not
        // claim the retry window refreshed them.
        assert_eq!(snapshot.refreshed_at, read_at);
        assert_eq!(snapshot.warnings[0].id, "claude.rateLimitedStale");
        assert_eq!(
            snapshot.notices[0]
                .params
                .get("rememberedAt")
                .map(String::as_str),
            Some(read_at.to_rfc3339().as_str())
        );
    }

    #[test]
    fn a_rate_limited_refresh_reuses_the_limits_the_last_run_left_on_disk() {
        let directory = tempdir().unwrap();
        let storage = Arc::new(Storage::open(&directory.path().join("openquota.db")).unwrap());
        let read_at = Utc::now() - Duration::minutes(7);
        let persisted = remembered_limit_snapshot(64.0, read_at, true);
        storage.save_snapshot(&persisted).unwrap();

        // A cold start: this process has read nothing itself, only what the last one saved.
        let provider = ClaudeProvider::new_scoped(
            ClaudeRuntimeConfig {
                definition: definition(),
                credential_scope: ClaudeCredentialScope::Standard,
                account_identity: None,
            },
            ClaudeClient::new().unwrap(),
            Some(Arc::clone(&storage)),
        );
        let restored = provider.last_good.lock().unwrap().clone().unwrap();
        assert!(restored.remembered);
        // A persisted rate limit window must not leak its own presentation into the next one.
        assert!(restored.notices.is_empty());
        assert!(restored.warnings.is_empty());
        assert_eq!(restored.quotas[0].used_percent, 64.0);
        assert_eq!(restored.refreshed_at, read_at);

        let snapshot = provider.rate_limited_snapshot(None, 300, Vec::new());

        assert_eq!(snapshot.quotas[0].used_percent, 64.0);
        assert_eq!(snapshot.plan, persisted.plan);
        // The numbers are only as fresh as the read they came from, so the card must not
        // claim the retry window refreshed them.
        assert_eq!(snapshot.refreshed_at, read_at);
        assert!(snapshot.remembered);
        assert_eq!(snapshot.notices[0].id, "claude.rateLimited");
        assert_eq!(
            snapshot.notices[0].message,
            "Showing the last successful limits · Retrying in about 5 minutes"
        );
        assert_eq!(snapshot.warnings.len(), 1);
        assert_eq!(snapshot.warnings[0].id, "claude.rateLimitedStale");
    }

    #[test]
    fn the_first_live_usage_activation_keeps_the_limits_restored_from_disk() {
        let directory = tempdir().unwrap();
        let storage = Arc::new(Storage::open(&directory.path().join("openquota.db")).unwrap());
        let read_at = Utc::now() - Duration::minutes(7);
        storage
            .save_snapshot(&remembered_limit_snapshot(64.0, read_at, false))
            .unwrap();

        let provider = ClaudeProvider::new_scoped(
            ClaudeRuntimeConfig {
                definition: definition(),
                credential_scope: ClaudeCredentialScope::Standard,
                account_identity: None,
            },
            ClaudeClient::new().unwrap(),
            Some(Arc::clone(&storage)),
        );

        // A refresh activates its credential before it checks the rate limit window, so an
        // activation that dropped the restored limits would empty the very card this fixes.
        provider.activate_live_usage_cache([7; 32]);
        assert_eq!(
            provider
                .last_good
                .lock()
                .unwrap()
                .as_ref()
                .map(|snapshot| snapshot.quotas[0].used_percent),
            Some(64.0)
        );

        // A later credential swap still discards them, because they belong to the old login.
        provider.activate_live_usage_cache([9; 32]);
        assert!(provider.last_good.lock().unwrap().is_none());
    }

    #[test]
    fn a_rate_limited_refresh_reports_no_data_when_nothing_was_ever_saved() {
        let directory = tempdir().unwrap();
        let storage = Arc::new(Storage::open(&directory.path().join("openquota.db")).unwrap());

        let provider = ClaudeProvider::new_scoped(
            ClaudeRuntimeConfig {
                definition: definition(),
                credential_scope: ClaudeCredentialScope::Standard,
                account_identity: None,
            },
            ClaudeClient::new().unwrap(),
            Some(storage),
        );
        assert!(provider.last_good.lock().unwrap().is_none());

        let snapshot = provider.rate_limited_snapshot(None, 300, Vec::new());

        assert!(snapshot.quotas.is_empty());
        assert!(snapshot.remembered);
        assert_eq!(snapshot.notices[0].message, "Retrying in about 5 minutes");
        assert_eq!(snapshot.warnings[0].id, "claude.rateLimitedRetry");
    }

    #[test]
    fn account_change_clears_live_usage_and_rate_limit_cache() {
        let provider = ClaudeProvider::new();
        let snapshot = ProviderSnapshot {
            provider_id: "claude".into(),
            plan: None,
            quotas: Vec::new(),
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: Utc::now(),
            remembered: false,
        };

        provider.activate_live_usage_cache([1; 32]);
        *provider.last_good.lock().unwrap() = Some(snapshot);
        *provider.rate_limited_until.lock().unwrap() = Some(Utc::now() + Duration::minutes(5));
        provider.activate_live_usage_cache([1; 32]);
        assert!(provider.last_good.lock().unwrap().is_some());
        assert!(provider.rate_limited_until.lock().unwrap().is_some());

        provider.activate_live_usage_cache([2; 32]);
        assert!(provider.last_good.lock().unwrap().is_none());
        assert!(provider.rate_limited_until.lock().unwrap().is_none());
    }

    #[test]
    fn login_changed_during_usage_request_is_not_published_under_the_old_card() {
        let directory = tempdir().unwrap();
        let account_root = directory.path().join("account");
        fs::create_dir_all(&account_root).unwrap();
        let credential_path = account_root.join(".credentials.json");
        let identity_path = account_root.join(".claude.json");
        fs::write(
            &credential_path,
            credential_json("account-a", "refresh-a", "pro"),
        )
        .unwrap();
        fs::write(
            &identity_path,
            r#"{"oauthAccount":{"accountUuid":"account-a"}}"#,
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (first_request_tx, first_request_rx) = mpsc::sync_channel(0);
        let (first_response_tx, first_response_rx) = mpsc::sync_channel(0);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0_u8; 4096];
            let length = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..length]);
            let authorization = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
                .unwrap();
            first_request_tx.send(()).unwrap();
            first_response_rx.recv().unwrap();
            write_http_response(&mut stream, 25);
            authorization
        });

        let credential_scope = ClaudeCredentialScope::ConfigDir {
            path: account_root.clone(),
            keychain_literal: account_root.to_string_lossy().into_owned(),
        };
        let provider = Arc::new(ClaudeProvider::new_scoped(
            ClaudeRuntimeConfig {
                definition: definition(),
                account_identity: accounts::identity_for_scope(&credential_scope),
                credential_scope,
            },
            ClaudeClient::new().unwrap(),
            None,
        ));
        let config = ClaudeOAuthConfig {
            usage_url: format!("{base}/usage"),
            refresh_url: format!("{base}/token"),
            client_id: "test-client".into(),
        };
        let refresh = thread::spawn(move || provider.refresh_inner_with_config(&config));

        first_request_rx
            .recv_timeout(StdDuration::from_secs(2))
            .unwrap();
        fs::write(
            &credential_path,
            credential_json("account-b", "refresh-b", "max"),
        )
        .unwrap();
        fs::write(
            &identity_path,
            r#"{"oauthAccount":{"accountUuid":"account-b"}}"#,
        )
        .unwrap();
        first_response_tx.send(()).unwrap();

        let error = refresh.join().unwrap().unwrap_err();
        let authorization = server.join().unwrap();
        assert!(matches!(error, ClaudeError::AccountChanged));
        assert_eq!(authorization, "Bearer account-a");
    }
}
