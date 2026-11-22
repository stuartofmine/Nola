// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, LazyLock},
};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_window_state::{StateFlags, WindowExt};

const TRAY_ID: &str = "nola-tray";

static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/locales.json")))
        .expect("checked desktop translations")
});

pub fn normalize_locale(raw: &str) -> &'static str {
    let language = raw.trim().to_lowercase().replace('_', "-");
    if language.starts_with("zh") {
        if ["zh-hant", "zh-tw", "zh-hk", "zh-mo"]
            .iter()
            .any(|prefix| language.starts_with(prefix))
        {
            return "zh-Hant";
        }
        return "zh-Hans";
    }
    match language.split('-').next().unwrap_or("en") {
        "de" => "de",
        "es" => "es",
        "fr" => "fr",
        "ja" => "ja",
        "ko" => "ko",
        "ru" => "ru",
        _ => "en",
    }
}

static LANGUAGE: LazyLock<String> = LazyLock::new(|| {
    let detected = sys_locale::get_locale().unwrap_or_default();
    normalize_locale(&detected).into()
});

pub fn language() -> &'static str {
    LANGUAGE.as_str()
}

pub fn label(key: &str) -> &'static str {
    label_for(None, key)
}

pub fn label_for(locale: Option<&str>, key: &str) -> &'static str {
    let lang = locale
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_locale)
        .unwrap_or_else(|| LANGUAGE.as_str());
    CATALOG[lang][key]
        .as_str()
        .expect("checked desktop translation key")
}

pub fn theme_background(theme: tauri::Theme) -> tauri::window::Color {
    match theme {
        tauri::Theme::Dark => tauri::window::Color(0x0b, 0x0b, 0x0c, 0xff),
        _ => tauri::window::Color(0xfa, 0xfa, 0xfa, 0xff),
    }
}

fn theme_preference_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = std::env::var_os("NOLA_E2E_USER_DATA")
        .map(PathBuf::from)
        .or_else(|| app.path().app_local_data_dir().ok())?;
    Some(dir.join("theme-preference.txt"))
}

pub fn load_persisted_theme(app: &AppHandle) -> (String, Option<tauri::Theme>) {
    let raw = theme_preference_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    match raw.trim() {
        "dark" => ("dark".into(), Some(tauri::Theme::Dark)),
        "light" => ("light".into(), Some(tauri::Theme::Light)),
        _ => ("system".into(), None),
    }
}

fn save_persisted_theme(app: &AppHandle, theme: &str) {
    if let Some(path) = theme_preference_path(app) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, theme);
    }
}

pub fn apply_theme_background(window: &WebviewWindow, explicit: Option<tauri::Theme>) {
    let effective = explicit
        .or_else(|| window.theme().ok())
        .unwrap_or(tauri::Theme::Light);
    let _ = window.set_background_color(Some(theme_background(effective)));
}

fn require_host(window: &WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|e| e.to_string())?;
    if window.label() == "main"
        && window
            .state::<super::Desktop>()
            .origin
            .lock()
            .unwrap()
            .as_ref()
            == Some(&url.origin())
    {
        Ok(())
    } else {
        Err("Desktop setting requires the local workspace".into())
    }
}

#[tauri::command]
pub fn native_runtime(window: WebviewWindow) -> Result<Value, String> {
    require_host(&window)?;
    let path = window.path().resource_dir().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(path.join("native-runtime.json")).map_err(|e| e.to_string())?;
    let runtime: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(json!({"architecture":runtime["architecture"],"backend":runtime["backend"]}))
}

pub fn login_item_available() -> bool {
    login_item_available_for_build(cfg!(debug_assertions))
}

fn login_item_available_for_build(debug_build: bool) -> bool {
    !debug_build
}

#[tauri::command]
pub fn login_item(window: WebviewWindow, locale: Option<String>) -> Result<Value, String> {
    require_host(&window)?;
    let available = login_item_available();
    let enabled = if available {
        window
            .autolaunch()
            .is_enabled()
            .map_err(|e| e.to_string())?
    } else {
        false
    };
    let requested = locale.as_deref();
    Ok(json!({"available":available,"enabled":enabled,"labels":{
        "title":label_for(requested, "startAtLogin"),
        "body":label_for(requested, "startAtLoginBody"),
        "enabled":label_for(requested, "enabled"),
        "disabled":label_for(requested, "disabled"),
        "unavailable":label_for(requested, "packagedOnly")
    }}))
}

#[tauri::command]
pub fn set_login_item(
    window: WebviewWindow,
    enabled: bool,
    locale: Option<String>,
) -> Result<Value, String> {
    require_host(&window)?;
    if !login_item_available() {
        return Err(label_for(locale.as_deref(), "packagedOnly").into());
    }
    let manager = window.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())?;
    login_item(window, locale)
}

#[tauri::command]
pub fn set_theme(window: WebviewWindow, theme: String) -> Result<(), String> {
    require_host(&window)?;
    let target = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        "system" => None,
        _ => return Err("Unsupported theme".into()),
    };
    if let Ok(mut pref) = window.state::<super::Desktop>().theme_preference.lock() {
        *pref = theme.clone();
    }
    save_persisted_theme(window.app_handle(), &theme);
    window.set_theme(target).map_err(|e| e.to_string())?;
    apply_theme_background(&window, target);
    Ok(())
}

#[tauri::command]
pub fn open_logs_folder(window: WebviewWindow) -> Result<(), String> {
    require_host(&window)?;
    let dir = window
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;
    open::that_detached(dir).map_err(|e| e.to_string())
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        apply_theme_background(&window, None);
        if app
            .state::<super::Desktop>()
            .pending_window_state_restore
            .swap(false, Ordering::SeqCst)
        {
            let _ = window.restore_state(
                StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
            );
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if app
            .state::<super::Desktop>()
            .failure
            .lock()
            .is_ok_and(|f| f.is_none())
        {
            set_tray_tooltip(app, label("appName"));
        }
    }
}

pub fn background_hint_tooltip(locale: Option<&str>) -> String {
    format!(
        "{} — {}",
        label_for(locale, "backgroundTitle"),
        label_for(locale, "backgroundBody")
    )
}

pub fn notify_background_hidden_once(app: &AppHandle) -> bool {
    let state = app.state::<super::Desktop>();
    if state.background_notice_shown.swap(true, Ordering::SeqCst) {
        return false;
    }
    if state.failure.lock().is_ok_and(|f| f.is_none()) {
        set_tray_tooltip(app, &background_hint_tooltip(None));
    }
    true
}

pub fn resolve_licenses_dir(resource_dir: &Path) -> Option<PathBuf> {
    let bundled = resource_dir.join("licenses");
    if bundled.is_dir() {
        return Some(bundled);
    }
    let fallback = resource_dir.join("resources").join("licenses");
    if fallback.is_dir() {
        return Some(fallback);
    }
    None
}

pub fn set_tray_tooltip(app: &AppHandle, tooltip: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

pub fn tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text("show", label("showNola"))
        .text("settings", label("settings"))
        .text("workspace", label("openWorkspace"))
        .text("logs", label("openLogFolder"))
        .text("licenses", label("thirdPartyLicenses"))
        .text("restart", label("restartEngine"))
        .separator()
        .text("quit", label("quit"))
        .build()?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(label("appName"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show(app),
            "settings" => {
                show(app);
                if let Some(window) = app.get_webview_window("main") {
                    if let Some(origin) = app
                        .state::<super::Desktop>()
                        .origin
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone())
                    {
                        if let Ok(url) = origin
                            .ascii_serialization()
                            .parse::<url::Url>()
                            .and_then(|base| base.join("/settings"))
                        {
                            let _ = window.navigate(url);
                        }
                    }
                }
            }
            "logs" => {
                if let Ok(dir) = app.path().app_local_data_dir() {
                    let _ = open::that_detached(dir);
                }
            }
            "licenses" => {
                if let Ok(dir) = app.path().resource_dir() {
                    if let Some(licenses_dir) = resolve_licenses_dir(&dir) {
                        let _ = open::that_detached(licenses_dir);
                    }
                }
            }
            "workspace" => {
                if let Ok(dir) = app.path().document_dir() {
                    let _ = open::that_detached(dir.join("Nola Workspace"));
                }
            }
            "restart" => {
                super::restart_host(app);
            }
            "quit" => {
                super::stop_host(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_locale_handles_all_supported_prefixes() {
        assert_eq!(normalize_locale("en-US"), "en");
        assert_eq!(normalize_locale("zh-CN"), "zh-Hans");
        assert_eq!(normalize_locale("zh_TW"), "zh-Hant");
        assert_eq!(normalize_locale("zh-Hant-HK"), "zh-Hant");
        assert_eq!(normalize_locale("ja-JP"), "ja");
        assert_eq!(normalize_locale("ko-KR"), "ko");
        assert_eq!(normalize_locale("de-DE"), "de");
        assert_eq!(normalize_locale("es-419"), "es");
        assert_eq!(normalize_locale("fr-FR"), "fr");
        assert_eq!(normalize_locale("ru-RU"), "ru");
        assert_eq!(normalize_locale("unknown"), "en");
    }

    #[test]
    fn label_for_respects_requested_locale_override() {
        assert_eq!(
            label_for(Some("zh-Hans"), "startAtLogin"),
            "登录 Windows 时启动 Nola"
        );
        assert_eq!(
            label_for(Some("ja"), "startAtLogin"),
            "Windows サインイン時に Nola を起動"
        );
        assert_eq!(
            label_for(Some("en"), "startAtLogin"),
            "Start Nola when I sign in"
        );
    }

    #[test]
    fn login_item_availability_disables_debug_builds_and_enables_packaged_builds() {
        assert!(!login_item_available_for_build(true));
        assert!(login_item_available_for_build(false));
        assert_eq!(login_item_available(), !cfg!(debug_assertions));
    }

    #[test]
    fn background_hint_tooltip_combines_localized_title_and_body() {
        let en = background_hint_tooltip(Some("en"));
        assert!(en.contains(label_for(Some("en"), "backgroundTitle")));
        assert!(en.contains(label_for(Some("en"), "backgroundBody")));
        let zh = background_hint_tooltip(Some("zh-Hans"));
        assert!(zh.contains(label_for(Some("zh-Hans"), "backgroundTitle")));
        assert!(zh.contains(label_for(Some("zh-Hans"), "backgroundBody")));
    }

    #[test]
    fn resolve_licenses_dir_finds_bundled_and_fallback_directories() {
        let root = std::env::temp_dir().join(format!("nola-licenses-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(resolve_licenses_dir(&root), None);

        let fallback = root.join("resources").join("licenses");
        std::fs::create_dir_all(&fallback).unwrap();
        assert_eq!(resolve_licenses_dir(&root), Some(fallback));

        let bundled = root.join("licenses");
        std::fs::create_dir_all(&bundled).unwrap();
        assert_eq!(resolve_licenses_dir(&root), Some(bundled));

        let _ = std::fs::remove_dir_all(&root);
    }
}
