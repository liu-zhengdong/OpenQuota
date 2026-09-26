use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    App, AppHandle, Manager, Runtime, Wry,
};

use crate::i18n::{t, UiLocale, UiLocaleState};

pub struct TrayMenuHandles {
    open: Option<MenuItem<Wry>>,
    customize: Option<MenuItem<Wry>>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl TrayMenuHandles {
    pub fn apply_locale(&self, locale: UiLocale) -> Result<(), String> {
        if let Some(open) = &self.open {
            open.set_text(t(locale, "tray.open"))
                .map_err(|_| "tray open label could not be updated".to_owned())?;
        }
        if let Some(customize) = &self.customize {
            customize
                .set_text(t(locale, "tray.customize"))
                .map_err(|_| "tray customize label could not be updated".to_owned())?;
        }
        let settings_key = if self.open.is_some() {
            "tray.settingsEllipsis"
        } else {
            "tray.settings"
        };
        self.settings
            .set_text(t(locale, settings_key))
            .map_err(|_| "tray settings label could not be updated".to_owned())?;
        self.quit
            .set_text(t(locale, "tray.quit"))
            .map_err(|_| "tray quit label could not be updated".to_owned())?;
        Ok(())
    }
}

pub fn build(app: &mut App, locale: UiLocale) -> Result<(Menu<Wry>, TrayMenuHandles), String> {
    #[cfg(target_os = "macos")]
    {
        let settings = menu_item(
            app,
            "settings",
            t(locale, "tray.settings"),
            Some("CmdOrCtrl+,"),
        )?;
        let quit = menu_item(app, "quit", t(locale, "tray.quit"), Some("CmdOrCtrl+Q"))?;
        let separator = PredefinedMenuItem::separator(app).map_err(menu_error)?;
        let menu = Menu::with_items(app, &[&settings, &separator, &quit]).map_err(menu_error)?;
        Ok((
            menu,
            TrayMenuHandles {
                open: None,
                customize: None,
                settings,
                quit,
            },
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let open = menu_item(app, "open", t(locale, "tray.open"), None)?;
        let customize = menu_item(app, "customize", t(locale, "tray.customize"), None)?;
        let settings = menu_item(app, "settings", t(locale, "tray.settingsEllipsis"), None)?;
        let quit = menu_item(app, "quit", t(locale, "tray.quit"), None)?;
        let separator = PredefinedMenuItem::separator(app).map_err(menu_error)?;
        let menu = Menu::with_items(app, &[&open, &customize, &settings, &separator, &quit])
            .map_err(menu_error)?;
        Ok((
            menu,
            TrayMenuHandles {
                open: Some(open),
                customize: Some(customize),
                settings,
                quit,
            },
        ))
    }
}

pub fn refresh_for_app(app: &AppHandle, locale: UiLocale) {
    if let Some(handles) = app.try_state::<TrayMenuHandles>() {
        if let Err(error) = handles.apply_locale(locale) {
            crate::app_warn!("tray", "tray menu locale update failed: {error}");
        }
    }
}

pub fn refresh_from_state(app: &AppHandle) {
    let Some(state) = app.try_state::<UiLocaleState>() else {
        return;
    };
    refresh_for_app(app, state.get());
}

fn menu_item<R: Runtime>(
    app: &App<R>,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
) -> Result<MenuItem<R>, String> {
    MenuItem::with_id(app, id, text, true, accelerator).map_err(menu_error)
}

fn menu_error(error: tauri::Error) -> String {
    error.to_string()
}
