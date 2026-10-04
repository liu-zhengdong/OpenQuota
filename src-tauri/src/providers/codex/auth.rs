use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::CodexError;

#[derive(Debug, Clone)]
pub struct CodexAuthState {
    pub access_token: String,
    pub account_id: Option<String>,
}

impl CodexAuthState {
    pub fn load_candidates() -> Result<Vec<Self>, CodexError> {
        let mut candidates = Vec::new();
        let mut api_key_only = false;
        for path in auth_paths() {
            if !path.is_file() {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(document) = parse_auth_document(&text) else {
                continue;
            };
            let access_token = document
                .pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if let Some(access_token) = access_token {
                candidates.push(Self {
                    account_id: string_at(&document, "/tokens/account_id"),
                    access_token,
                });
                continue;
            }
            api_key_only |= document
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty());
        }
        if let Some(state) = load_keychain_candidate() {
            candidates.push(state);
        }
        if !candidates.is_empty() {
            Ok(candidates)
        } else if api_key_only {
            Err(CodexError::ApiKeyOnly)
        } else {
            Err(CodexError::NotLoggedIn)
        }
    }
}

#[cfg(target_os = "macos")]
fn keychain_document() -> Option<Value> {
    let bytes = crate::providers::credential_store::read_generic_password("Codex Auth", "")
        .ok()
        .flatten()?;
    parse_auth_document(std::str::from_utf8(&bytes).ok()?)
}

#[cfg(target_os = "macos")]
fn load_keychain_candidate() -> Option<CodexAuthState> {
    let document = keychain_document()?;
    let access_token =
        string_at(&document, "/tokens/access_token").filter(|value| !value.is_empty())?;
    Some(CodexAuthState {
        account_id: string_at(&document, "/tokens/account_id"),
        access_token,
    })
}

#[cfg(not(target_os = "macos"))]
fn load_keychain_candidate() -> Option<CodexAuthState> {
    None
}

pub fn auth_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    candidate_paths(
        &home,
        crate::provider_environment::value("CODEX_HOME")
            .map(PathBuf::from)
            .as_deref(),
    )
}

fn candidate_paths(home: &Path, codex_home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(codex_home) = codex_home.filter(|path| !path.as_os_str().is_empty()) {
        return vec![codex_home.join("auth.json")];
    }
    vec![
        home.join(".config").join("codex").join("auth.json"),
        home.join(".codex").join("auth.json"),
    ]
}

fn parse_auth_document(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let trimmed = text.trim();
        if !trimmed.len().is_multiple_of(2) || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        let bytes = (0..trimmed.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16).ok())
            .collect::<Option<Vec<_>>>()?;
        serde_json::from_slice(&bytes).ok()
    })
}

fn string_at(document: &Value, pointer: &str) -> Option<String> {
    document
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{candidate_paths, parse_auth_document};

    #[test]
    fn codex_home_replaces_default_candidates() {
        assert_eq!(
            candidate_paths(Path::new("/users/me"), Some(Path::new("/custom/codex"))),
            vec![Path::new("/custom/codex/auth.json")]
        );
    }

    #[test]
    fn parses_hex_encoded_auth_without_exposing_tokens() {
        let raw = r#"{"tokens":{"access_token":"placeholder"}}"#;
        let hex = raw
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            parse_auth_document(&hex)
                .unwrap()
                .pointer("/tokens/access_token")
                .and_then(|value| value.as_str()),
            Some("placeholder")
        );
    }
}
