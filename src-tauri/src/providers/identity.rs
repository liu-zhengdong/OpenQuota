//! Provenance and comparison of the account hashes already used by provider caches.
use super::CacheIdentity;
use crate::models::AccountIdentity;

pub fn account_fact(provider_id: &str, value: Option<&str>) -> Option<AccountIdentity> {
    let source = match provider_id.split('@').next()? {
        "claude" => "claude:sha256(lowercase(accountUuid[|organizationUuid])); organization included only when observed",
        "codex" => "codex:sha256(lowercase(account_id or id_token.chatgpt_account_id)); no organization boundary observed",
        _ => return None,
    };
    Some(AccountIdentity {
        kind: "accountHash".into(),
        value: value?.into(),
        source: source.into(),
    })
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CacheIdentityMatch {
    Matched,
    Mismatched,
    #[default]
    Unknown,
}

pub fn cache_match(
    current: CacheIdentity<'_>,
    cached: Option<&AccountIdentity>,
) -> CacheIdentityMatch {
    match (current, cached) {
        (CacheIdentity::Resolved(current), Some(cached)) if current == cached.value => {
            CacheIdentityMatch::Matched
        }
        (CacheIdentity::Resolved(_), Some(_)) => CacheIdentityMatch::Mismatched,
        _ => CacheIdentityMatch::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_identity_is_required_and_credential_fingerprints_are_not_accounts() {
        let cached = account_fact("claude", Some("account-a")).unwrap();
        assert_eq!(
            cache_match(CacheIdentity::Resolved("account-a"), Some(&cached)),
            CacheIdentityMatch::Matched
        );
        assert_eq!(
            cache_match(CacheIdentity::Resolved("account-b"), Some(&cached)),
            CacheIdentityMatch::Mismatched
        );
        assert_eq!(
            cache_match(CacheIdentity::Unresolved, Some(&cached)),
            CacheIdentityMatch::Unknown
        );
        assert_eq!(
            cache_match(CacheIdentity::Unscoped, Some(&cached)),
            CacheIdentityMatch::Unknown
        );
        assert_eq!(
            cache_match(CacheIdentity::Resolved("account-a"), None),
            CacheIdentityMatch::Unknown
        );
        assert!(account_fact("opencode", Some("fingerprint")).is_none());
    }
}
