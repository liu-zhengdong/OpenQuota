use super::CommandCodeError;
use std::{fs, path::PathBuf};
use zeroize::Zeroizing;

pub struct CommandCodeAuth {
    path: Option<PathBuf>,
    #[cfg(test)]
    isolated: bool,
}
impl CommandCodeAuth {
    pub fn new() -> Self {
        Self {
            #[cfg(test)]
            isolated: false,
            path: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".commandcode/auth.json")),
        }
    }
    pub fn load(&self) -> Result<Option<Zeroizing<String>>, CommandCodeError> {
        #[cfg(test)]
        if self.isolated {
            return self.load_with_env(None);
        }
        self.load_with_env(crate::provider_environment::value("COMMAND_CODE_API_KEY"))
    }
    pub(super) fn load_with_env(
        &self,
        fallback: Option<String>,
    ) -> Result<Option<Zeroizing<String>>, CommandCodeError> {
        let file = self.path.as_ref().map(fs::read_to_string).transpose();
        let parsed = match file {
            Ok(Some(text)) => parse(&text),
            Ok(None) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(CommandCodeError::CredentialStorage),
        };
        match parsed {
            Ok(Some(key)) => Ok(Some(key)),
            result => match fallback.filter(|key| !key.trim().is_empty()) {
                Some(key) => Ok(Some(Zeroizing::new(key.trim().to_owned()))),
                None => result,
            },
        }
    }
    #[cfg(test)]
    pub(super) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            isolated: true,
        }
    }
}
fn parse(text: &str) -> Result<Option<Zeroizing<String>>, CommandCodeError> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|_| CommandCodeError::CredentialStorage)?;
    match doc.get("apiKey") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(key)) => {
            Ok((!key.trim().is_empty()).then(|| Zeroizing::new(key.trim().to_owned())))
        }
        _ => Err(CommandCodeError::CredentialStorage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_precedes_environment_and_never_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let document = r#"{"apiKey":"file-key","other":true}"#;
        fs::write(&path, document).unwrap();
        let auth = CommandCodeAuth::at(path.clone());
        assert_eq!(
            auth.load_with_env(Some("env-key".into()))
                .unwrap()
                .unwrap()
                .as_str(),
            "file-key"
        );
        assert_eq!(fs::read_to_string(path).unwrap(), document);
    }
    #[test]
    fn absent_empty_and_broken_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let auth = CommandCodeAuth::at(path.clone());
        assert!(auth.load_with_env(None).unwrap().is_none());
        assert_eq!(
            auth.load_with_env(Some(" env-key ".into()))
                .unwrap()
                .unwrap()
                .as_str(),
            "env-key"
        );
        for doc in ["{}", r#"{"apiKey":"  "}"#, r#"{"apiKey":null}"#] {
            fs::write(&path, doc).unwrap();
            assert!(auth.load_with_env(None).unwrap().is_none());
        }
        for doc in ["broken", r#"{"apiKey":42}"#] {
            fs::write(&path, doc).unwrap();
            assert!(matches!(
                auth.load_with_env(None),
                Err(CommandCodeError::CredentialStorage)
            ));
            assert!(auth
                .load_with_env(Some("env-key".into()))
                .unwrap()
                .is_some());
        }
    }
}
