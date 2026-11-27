// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, LazyLock},
};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_window_state::{StateFlags, WindowExt};

const TRAY_ID: &str = "nola-tray";
pub const TRAY_ACTION_ERROR_EVENT: &str = "nola://tray-action-error";
const MAX_DESKTOP_ACTION_LOG_BYTES: usize = 16 * 1024;

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

pub fn desktop_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("NOLA_E2E_USER_DATA") {
        return Ok(PathBuf::from(dir));
    }
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}

pub fn desktop_workspace_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = if let Some(dir) = std::env::var_os("NOLA_E2E_USER_DATA") {
        PathBuf::from(dir)
    } else {
        app.path().document_dir().map_err(|e| e.to_string())?
    };
    Ok(base.join("Nola Workspace"))
}

fn theme_preference_path(app: &AppHandle) -> Option<PathBuf> {
    Some(desktop_data_dir(app).ok()?.join("theme-preference.txt"))
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
        && super::lock_unpoisoned(&window.state::<super::Desktop>().origin).as_ref()
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
    *super::lock_unpoisoned(&window.state::<super::Desktop>().theme_preference) = theme.clone();
    save_persisted_theme(window.app_handle(), &theme);
    window.set_theme(target).map_err(|e| e.to_string())?;
    apply_theme_background(&window, target);
    Ok(())
}

#[tauri::command]
pub fn open_logs_folder(window: WebviewWindow) -> Result<(), String> {
    require_host(&window)?;
    let dir = desktop_data_dir(window.app_handle())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open::that_detached(dir).map_err(|e| e.to_string())
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let explicit_theme = match super::lock_unpoisoned(&app.state::<super::Desktop>().theme_preference).as_str() {
            "dark" => Some(tauri::Theme::Dark),
            "light" => Some(tauri::Theme::Light),
            _ => None,
        };
        apply_theme_background(&window, explicit_theme);
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
        if super::lock_unpoisoned(&app.state::<super::Desktop>().failure).is_none() {
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
    if super::lock_unpoisoned(&state.failure).is_none() {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayFolderAction {
    Logs,
    Licenses,
    Workspace,
}

impl TrayFolderAction {
    pub fn id(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Licenses => "licenses",
            Self::Workspace => "workspace",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::Logs => "openLogFolder",
            Self::Licenses => "thirdPartyLicenses",
            Self::Workspace => "openWorkspace",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayActionError {
    pub action: String,
    pub message: String,
    pub detail: String,
}

impl TrayActionError {
    pub fn to_json(&self) -> Value {
        json!({
            "action": self.action,
            "message": self.message,
            "detail": self.detail,
        })
    }
}

pub fn format_tray_open_error(action: TrayFolderAction, locale: Option<&str>) -> String {
    format!(
        "{} — {}",
        label_for(locale, action.label_key()),
        label_for(locale, "openFolderFailed")
    )
}

pub fn format_tray_licenses_missing_error(locale: Option<&str>) -> String {
    format!(
        "{} — {}",
        label_for(locale, TrayFolderAction::Licenses.label_key()),
        label_for(locale, "licensesUnavailable")
    )
}

pub fn open_tray_folder_with<F, O>(
    action: TrayFolderAction,
    locale: Option<&str>,
    resolve_dir: F,
    create_if_missing: bool,
    open_path: O,
) -> Result<PathBuf, TrayActionError>
where
    F: FnOnce() -> Result<Option<PathBuf>, String>,
    O: FnOnce(&Path) -> Result<(), String>,
{
    let dir = match resolve_dir() {
        Ok(Some(dir)) => dir,
        Ok(None) => {
            let (message, detail) = match action {
                TrayFolderAction::Licenses => (
                    format_tray_licenses_missing_error(locale),
                    "Bundled third-party licenses directory was not found".into(),
                ),
                _ => (
                    format_tray_open_error(action, locale),
                    "Target directory is unavailable".into(),
                ),
            };
            return Err(TrayActionError {
                action: action.id().into(),
                message,
                detail,
            });
        }
        Err(err) => {
            return Err(TrayActionError {
                action: action.id().into(),
                message: format_tray_open_error(action, locale),
                detail: format!("resolve directory failed: {err}"),
            });
        }
    };

    if create_if_missing {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return Err(TrayActionError {
                action: action.id().into(),
                message: format_tray_open_error(action, locale),
                detail: format!("create_dir_all failed: {err}"),
            });
        }
    }

    if let Err(err) = open_path(&dir) {
        return Err(TrayActionError {
            action: action.id().into(),
            message: format_tray_open_error(action, locale),
            detail: format!("open failed: {err}"),
        });
    }

    Ok(dir)
}

pub fn log_tray_action_error(log_dir: Option<&Path>, error: &TrayActionError) {
    let entry = format!(
        "[nola-desktop] tray action '{}' failed: {} ({})",
        error.action, error.message, error.detail
    );
    eprintln!("{entry}");
    let Some(dir) = log_dir else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let log_path = dir.join("desktop-actions.log");
    let current_len = std::fs::metadata(&log_path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    if current_len + entry.len() + 1 > MAX_DESKTOP_ACTION_LOG_BYTES {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(file, "{entry}");
    }
}

pub fn tray_error_dispatch_script(error: &TrayActionError) -> String {
    let payload = serde_json::to_string(&error.to_json())
        .unwrap_or_else(|_| "{}".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "window.dispatchEvent(new CustomEvent('nola:desktop-error', {{ detail: {payload} }}));"
    )
}

pub fn clear_tray_action_error(app: &AppHandle) {
    let state = app.state::<super::Desktop>();
    *super::lock_unpoisoned(&state.tray_error) = None;
    if super::lock_unpoisoned(&state.failure).is_none() {
        set_tray_tooltip(app, label("appName"));
    }
}

pub fn report_tray_action_error(app: &AppHandle, error: TrayActionError) {
    log_tray_action_error(desktop_data_dir(app).ok().as_deref(), &error);
    {
        let state = app.state::<super::Desktop>();
        *super::lock_unpoisoned(&state.tray_error) = Some(error.clone());
    }
    let _ = app.emit(TRAY_ACTION_ERROR_EVENT, error.to_json());
    show(app);
    set_tray_tooltip(app, &format!("{} — {}", label("appName"), error.message));
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(&tray_error_dispatch_script(&error));
    }
}

fn handle_tray_folder_action(app: &AppHandle, action: TrayFolderAction) {
    let result = match action {
        TrayFolderAction::Logs => open_tray_folder_with(
            action,
            None,
            || desktop_data_dir(app).map(Some),
            true,
            |path| open::that_detached(path).map_err(|e| e.to_string()),
        ),
        TrayFolderAction::Licenses => open_tray_folder_with(
            action,
            None,
            || {
                let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
                Ok(resolve_licenses_dir(&resource_dir))
            },
            false,
            |path| open::that_detached(path).map_err(|e| e.to_string()),
        ),
        TrayFolderAction::Workspace => open_tray_folder_with(
            action,
            None,
            || desktop_workspace_dir(app).map(Some),
            true,
            |path| open::that_detached(path).map_err(|e| e.to_string()),
        ),
    };
    match result {
        Ok(_) => clear_tray_action_error(app),
        Err(error) => report_tray_action_error(app, error),
    }
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
                    if let Some(origin) = super::lock_unpoisoned(&app.state::<super::Desktop>().origin).clone() {
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
            "logs" => handle_tray_folder_action(app, TrayFolderAction::Logs),
            "licenses" => handle_tray_folder_action(app, TrayFolderAction::Licenses),
            "workspace" => handle_tray_folder_action(app, TrayFolderAction::Workspace),
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

    #[test]
    fn open_tray_folder_surfaces_open_failures_for_logs_licenses_and_workspace() {
        let root = std::env::temp_dir().join(format!("nola-tray-open-err-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        for (action, create) in [
            (TrayFolderAction::Logs, true),
            (TrayFolderAction::Workspace, true),
            (TrayFolderAction::Licenses, false),
        ] {
            let target = root.join(action.id());
            if !create {
                std::fs::create_dir_all(&target).unwrap();
            }
            let err = open_tray_folder_with(
                action,
                Some("en"),
                || Ok(Some(target.clone())),
                create,
                |_| Err("ShellExecute failed".into()),
            )
            .unwrap_err();
            assert_eq!(err.action, action.id());
            assert_eq!(err.message, format_tray_open_error(action, Some("en")));
            assert!(err.detail.contains("ShellExecute failed"));
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_tray_folder_licenses_surfaces_clear_error_when_missing() {
        let root = std::env::temp_dir().join(format!("nola-tray-lic-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut opener_called = false;
        let err = open_tray_folder_with(
            TrayFolderAction::Licenses,
            Some("en"),
            || Ok(resolve_licenses_dir(&root)),
            false,
            |_| {
                opener_called = true;
                Ok(())
            },
        )
        .unwrap_err();
        assert!(!opener_called);
        assert_eq!(err.action, "licenses");
        assert_eq!(err.message, format_tray_licenses_missing_error(Some("en")));
        assert!(err.message.contains(label_for(Some("en"), "licensesUnavailable")));
        assert!(err.detail.contains("not found"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_tray_folder_surfaces_resolve_and_create_dir_errors() {
        let resolve_err = open_tray_folder_with(
            TrayFolderAction::Logs,
            Some("en"),
            || Err("app_local_data_dir unavailable".into()),
            true,
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(resolve_err.action, "logs");
        assert_eq!(
            resolve_err.message,
            format_tray_open_error(TrayFolderAction::Logs, Some("en"))
        );
        assert!(resolve_err.detail.contains("app_local_data_dir unavailable"));

        let root = std::env::temp_dir().join(format!("nola-tray-create-err-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let blocker = root.join("blocker-file");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let blocked_child = blocker.join("workspace");

        let create_err = open_tray_folder_with(
            TrayFolderAction::Workspace,
            Some("en"),
            || Ok(Some(blocked_child)),
            true,
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(create_err.action, "workspace");
        assert!(create_err.detail.contains("create_dir_all failed"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_tray_folder_succeeds_and_creates_directory_when_requested() {
        let root = std::env::temp_dir().join(format!("nola-tray-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join("Nola Workspace");

        let mut opened = None;
        let result = open_tray_folder_with(
            TrayFolderAction::Workspace,
            Some("en"),
            || Ok(Some(target.clone())),
            true,
            |path| {
                opened = Some(path.to_path_buf());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(result, target);
        assert_eq!(opened.as_deref(), Some(target.as_path()));
        assert!(target.is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn log_tray_action_error_writes_bounded_log_and_dispatch_script_escapes_payload() {
        let root = std::env::temp_dir().join(format!("nola-tray-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let error = TrayActionError {
            action: "licenses".into(),
            message: "Third-party licenses — <script>alert(\"x\")</script>\u{2028}line".into(),
            detail: "Bundled third-party licenses directory was not found".into(),
        };
        log_tray_action_error(Some(&root), &error);
        let content = std::fs::read_to_string(root.join("desktop-actions.log")).unwrap();
        assert!(content.contains("tray action 'licenses' failed"));
        assert!(content.contains("Bundled third-party licenses directory was not found"));

        let script = tray_error_dispatch_script(&error);
        assert!(script.starts_with("window.dispatchEvent(new CustomEvent('nola:desktop-error'"));
        assert!(!script.contains('\u{2028}'));
        assert!(script.contains("\\u2028"));
        assert!(script.contains("\\\"x\\\""));

        let _ = std::fs::remove_dir_all(&root);
    }
}
