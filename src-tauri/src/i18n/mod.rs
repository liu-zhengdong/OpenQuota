mod catalog;
mod locale;

use std::sync::Mutex;

pub use catalog::{t, window_label};
pub use locale::{resolve_ui_locale, system_language_tag, UiLocale};

use crate::models::UiLanguagePreference;

pub struct UiLocaleState {
    resolved: Mutex<UiLocale>,
}

impl UiLocaleState {
    pub fn from_preference(preference: UiLanguagePreference) -> Self {
        Self {
            resolved: Mutex::new(resolve_ui_locale(preference, &system_language_tag())),
        }
    }

    pub fn get(&self) -> UiLocale {
        self.resolved
            .lock()
            .map(|locale| *locale)
            .unwrap_or(UiLocale::En)
    }

    pub fn set(&self, locale: UiLocale) -> bool {
        let Ok(mut current) = self.resolved.lock() else {
            return false;
        };
        if *current == locale {
            return false;
        }
        *current = locale;
        true
    }
}
