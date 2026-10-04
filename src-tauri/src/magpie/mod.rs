mod mapper;
#[cfg(test)]
mod tests;
#[cfg(test)]
use crate::providers;

use crate::{
    models::{ProviderDefinition, ProviderErrorKind, ProviderSnapshot},
    providers::{CacheIdentity, ProviderError, UsageProvider},
};
use mapper::Quota;
use serde::Deserialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Report {
    object: String,
    data: Vec<Quota>,
}

// A refresh wave shares one bounded report, including credential detection.
// This cache never hides a failed request behind older readings.
pub(crate) struct Source {
    client: reqwest::blocking::Client,
    url: String,
    report: Mutex<Option<(Instant, Vec<Quota>)>>,
}
impl Source {
    fn new(address: &str) -> Result<Self, ProviderError> {
        let url = reqwest::Url::parse(&format!("http://{address}/v1/magpie/quotas"))
            .map_err(|_| invalid("Invalid MAGPIE_ADDR."))?;
        if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/v1/magpie/quotas"
            || url.query().is_some()
        {
            return Err(invalid("MAGPIE_ADDR must be a loopback host and port."));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid("Could not create magpie client."))?;
        Ok(Self {
            client,
            url: url.to_string(),
            report: Mutex::new(None),
        })
    }
    fn read(&self) -> Result<Vec<Quota>, ProviderError> {
        let mut cache = self
            .report
            .lock()
            .map_err(|_| invalid("Magpie report lock unavailable."))?;
        if let Some((at, report)) = cache.as_ref() {
            if at.elapsed() < Duration::from_secs(2) {
                return Ok(report.clone());
            }
        }
        *cache = None;
        let response = self.client.get(&self.url).send().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Network,
                "Could not connect to magpie. Start its gateway.",
            )
        })?;
        if !response.status().is_success() {
            return Err(ProviderError::new(
                ProviderErrorKind::Network,
                format!(
                    "Magpie quota request failed (HTTP {}).",
                    response.status().as_u16()
                ),
            ));
        }
        // Read a bounded report; never put response bodies (account facts) in errors.
        use std::io::Read;
        let mut bytes = Vec::new();
        response
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("Could not read magpie report."))?;
        if bytes.len() > 1_048_576 {
            return Err(invalid("Magpie report exceeds 1 MiB."));
        }
        let report: Report =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid magpie quota report."))?;
        if report.object != "list" {
            return Err(invalid("Invalid magpie quota report object."));
        }
        *cache = Some((Instant::now(), report.data.clone()));
        Ok(report.data)
    }
}

struct MagpieProvider {
    definition: ProviderDefinition,
    source: Arc<Source>,
}
impl UsageProvider for MagpieProvider {
    fn definition(&self) -> ProviderDefinition {
        self.definition.clone()
    }
    fn has_local_credentials(&self) -> bool {
        self.source
            .read()
            .is_ok_and(|report| mapper::select(&report, &self.definition.id).is_ok())
    }
    fn refresh(&self) -> Result<ProviderSnapshot, ProviderError> {
        let report = self.source.read()?;
        mapper::snapshot(
            mapper::select(&report, &self.definition.id)?,
            &self.definition.id,
        )
    }
    // Vendor cache identities cannot establish which magpie account is selected.
    fn cache_identity(&self) -> CacheIdentity<'_> {
        CacheIdentity::Unresolved
    }
}

impl Source {
    pub(crate) fn from_environment() -> Result<Arc<Self>, ProviderError> {
        let address = crate::provider_environment::value("MAGPIE_ADDR")
            .unwrap_or_else(|| "127.0.0.1:3425".into());
        Ok(Arc::new(Self::new(&address)?))
    }
    pub(crate) fn runtime(
        self: &Arc<Self>,
        mut definition: ProviderDefinition,
    ) -> Arc<dyn UsageProvider> {
        definition.scoped_quota_prefix = Some("magpie-".into());
        Arc::new(MagpieProvider {
            definition,
            source: self.clone(),
        })
    }
}
#[cfg(test)]
fn runtimes_at(address: &str) -> Result<Vec<Arc<dyn UsageProvider>>, ProviderError> {
    let source = Arc::new(Source::new(address)?);
    Ok([
        providers::codex::definition(),
        providers::grok::definition(),
        providers::kimi::definition(),
        providers::zai::definition(),
        providers::opencode::definition(),
    ]
    .into_iter()
    .map(|definition| source.runtime(definition))
    .collect())
}
fn invalid(message: &str) -> ProviderError {
    ProviderError::new(ProviderErrorKind::InvalidResponse, message)
}

#[cfg(test)]
pub(crate) fn sample_snapshots() -> Vec<ProviderSnapshot> {
    tests::sample_snapshots()
}
