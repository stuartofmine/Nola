// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
use std::sync::LazyLock;
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt;

static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/locales.json")))
        .expect("checked desktop translations")
});
static LANGUAGE: LazyLock<String> = LazyLock::new(|| {
    let language = sys_locale::get_locale()
        .unwrap_or_default()
        .to_lowercase()
        .replace('_', "-");
    if language.starts_with("zh") {
        if ["zh-hant", "zh-tw", "zh-hk", "zh-mo"]
            .iter()
            .any(|prefix| language.starts_with(prefix))
        {
            return "zh-Hant".into();
        }
        return "zh-Hans".into();
    }
    let primary = language.split('-').next().unwrap_or("en");
    if CATALOG.get(primary).is_some() {
        primary.into()
    } else {
        "en".into()
    }
});
pub fn language() -> &'static str {
    LANGUAGE.as_str()
}

pub fn label(key: &str) -> &'static str {
    CATALOG[LANGUAGE.as_str()][key]
        .as_str()
        .expect("checked desktop translation key")
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

#[tauri::command]
pub fn login_item(window: WebviewWindow) -> Result<Value, String> {
    require_host(&window)?;
    let enabled = window
        .autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())?;
    Ok(json!({"available":true,"enabled":enabled,"labels":{
        "title":label("startAtLogin"),"body":label("startAtLoginBody"),"enabled":label("enabled"),
        "disabled":label("disabled"),"unavailable":label("packagedOnly")
    }}))
}
#[tauri::command]
pub fn set_login_item(window: WebviewWindow, enabled: bool) -> Result<Value, String> {
    require_host(&window)?;
    let manager = window.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())?;
    login_item(window)
}
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
pub fn tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text("show", label("showNola"))
        .text("workspace", label("openWorkspace"))
        .text("logs", label("openLogFolder"))
        .text("restart", label("restartEngine"))
        .separator()
        .text("quit", label("quit"))
        .build()?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("Nola")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show(app),
            "logs" => {
                if let Ok(dir) = app.path().app_local_data_dir() {
                    let _ = open::that_detached(dir);
                }
            }
            "workspace" => {
                if let Ok(dir) = app.path().document_dir() {
                    let _ = open::that_detached(dir.join("Nola Workspace"));
                }
            }
            "restart" => app.restart(),
            "quit" => app.exit(0),
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
