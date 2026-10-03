use super::CommandCodeError;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{thread, time::Duration};

pub struct CommandCodeClient {
    client: Client,
    base: String,
}
pub struct UsageData {
    pub credits: Value,
    pub subscription: Value,
    pub summary: Value,
}
impl CommandCodeClient {
    pub fn new() -> Result<Self, CommandCodeError> {
        Self::with_base("https://api.commandcode.ai", Duration::from_secs(15))
    }
    pub(super) fn with_base(base: &str, timeout: Duration) -> Result<Self, CommandCodeError> {
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(8))
                .timeout(timeout)
                .user_agent(concat!("OpenQuota/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| CommandCodeError::ConnectionFailed)?,
        })
    }
    fn get(
        &self,
        key: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, CommandCodeError> {
        let mut url = reqwest::Url::parse(&format!("{}{}", self.base, path))
            .map_err(|_| CommandCodeError::InvalidResponse)?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        let response = self
            .client
            .get(url)
            .bearer_auth(key)
            .header("Accept", "application/json")
            .send()
            .map_err(|_| CommandCodeError::ConnectionFailed)?;
        let status = response.status();
        if !status.is_success() {
            return Err(CommandCodeError::RequestFailed(status.as_u16()));
        }
        response
            .json()
            .map_err(|_| CommandCodeError::InvalidResponse)
    }
    pub fn fetch(&self, key: &str) -> Result<UsageData, CommandCodeError> {
        let whoami = self.get(key, "/alpha/whoami", &[("limits", "1")])?;
        // Personal accounts have no org. The CLI omits orgId in that case.
        let org = whoami
            .pointer("/org/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        if org.is_none()
            && whoami
                .pointer("/user/id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .is_none()
        {
            return Err(CommandCodeError::InvalidResponse);
        }
        let org_query: Vec<_> = org.map(|id| vec![("orgId", id)]).unwrap_or_default();
        // Summary depends on the subscription's period start; only these two requests are independent.
        let (credits, subscription) = thread::scope(|scope| {
            let credits = scope.spawn(|| self.get(key, "/alpha/billing/credits", &org_query));
            let subscription = self.get(key, "/alpha/billing/subscriptions", &org_query);
            let credits = credits
                .join()
                .map_err(|_| CommandCodeError::InvalidResponse)?;
            Ok::<_, CommandCodeError>((credits?, subscription?))
        })?;
        let mut query = org_query;
        if let Some(since) = subscription
            .pointer("/data/currentPeriodStart")
            .and_then(Value::as_str)
        {
            query.push(("since", since));
        }
        let summary = self.get(key, "/alpha/usage/summary", &query)?;
        Ok(UsageData {
            credits,
            subscription,
            summary,
        })
    }
}
