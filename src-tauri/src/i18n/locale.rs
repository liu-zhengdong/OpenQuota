use crate::models::UiLanguagePreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiLocale {
    En,
    Zh,
}

impl UiLocale {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "en" => Ok(Self::En),
            "zh" => Ok(Self::Zh),
            other => Err(format!("unsupported ui locale `{other}`")),
        }
    }
}

pub fn is_chinese_language_tag(tag: &str) -> bool {
    let normalized = tag.trim().to_ascii_lowercase().replace('_', "-");
    normalized == "zh" || normalized.starts_with("zh-")
}

pub fn resolve_ui_locale(preference: UiLanguagePreference, system_tag: &str) -> UiLocale {
    match preference {
        UiLanguagePreference::En => UiLocale::En,
        UiLanguagePreference::Zh => UiLocale::Zh,
        UiLanguagePreference::System => {
            if is_chinese_language_tag(system_tag) {
                UiLocale::Zh
            } else {
                UiLocale::En
            }
        }
    }
}

pub fn system_language_tag() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{is_chinese_language_tag, resolve_ui_locale, UiLocale};
    use crate::models::UiLanguagePreference;

    #[test]
    fn system_preference_uses_zh_prefix() {
        assert_eq!(
            resolve_ui_locale(UiLanguagePreference::System, "zh-CN"),
            UiLocale::Zh
        );
        assert_eq!(
            resolve_ui_locale(UiLanguagePreference::System, "zh"),
            UiLocale::Zh
        );
        assert_eq!(
            resolve_ui_locale(UiLanguagePreference::System, "en-US"),
            UiLocale::En
        );
        assert_eq!(
            resolve_ui_locale(UiLanguagePreference::En, "zh-CN"),
            UiLocale::En
        );
        assert_eq!(
            resolve_ui_locale(UiLanguagePreference::Zh, "en-US"),
            UiLocale::Zh
        );
    }

    #[test]
    fn chinese_tags_accept_underscore_and_script() {
        assert!(is_chinese_language_tag("zh_CN"));
        assert!(is_chinese_language_tag("zh-Hans-CN"));
        assert!(!is_chinese_language_tag("en-GB"));
    }
}
