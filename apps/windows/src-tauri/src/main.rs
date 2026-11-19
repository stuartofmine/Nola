// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;
mod native;

use std::{sync::Mutex, thread, time::Duration};
use tauri::{webview::NewWindowResponse, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;

#[derive(Default)]
struct Desktop {
    origin: Mutex<Option<url::Origin>>,
    failure: Mutex<Option<String>>,
}

fn local_page(url: &Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}

#[tauri::command]
fn startup_status(window: tauri::WebviewWindow, phase: String) -> Option<serde_json::Value> {
    if !local_page(&window.url().ok()?) {
        return None;
    }
    let state = window.state::<Desktop>();
    let failure = state.failure.lock().ok()?.clone();
    let current = if failure.is_some() {
        "failed"
    } else {
        "starting"
    };
    let page = (phase != current).then(|| {
        native::label(if failure.is_some() {
            "failedPage"
        } else {
            "startingPage"
        })
    });
    Some(
        serde_json::json!({"phase":current,"page":page,"language":native::language(),"failure":failure}),
    )
}

#[tauri::command]
fn open_diagnostics(window: tauri::WebviewWindow) -> Result<(), String> {
    if !window.url().is_ok_and(|url| local_page(&url)) {
        return Err("Unavailable".into());
    }
    open::that_detached(
        window
            .path()
            .app_local_data_dir()
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn retry_startup(window: tauri::WebviewWindow) {
    if window.url().is_ok_and(|url| local_page(&url))
        && window
            .state::<Desktop>()
            .failure
            .lock()
            .is_ok_and(|f| f.is_some())
    {
        window.app_handle().restart();
    }
}

fn start_host(app: &AppHandle, startup: Url) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Window closed")?;
    let path = app.path();
    let test_profile = std::env::var_os("NOLA_E2E_USER_DATA").map(std::path::PathBuf::from);
    let data = test_profile
        .clone()
        .map(Ok)
        .unwrap_or_else(|| path.app_local_data_dir())
        .map_err(|e| e.to_string())?;
    let workspace = test_profile
        .map(Ok)
        .unwrap_or_else(|| path.document_dir())
        .map_err(|e| e.to_string())?
        .join("Nola Workspace");
    let (mut child, origin) = host::Host::start(
        &path.resource_dir().map_err(|e| e.to_string())?,
        &data,
        &workspace,
    )?;
    let state = app.state::<Desktop>();
    app.add_capability(
        tauri::ipc::CapabilityBuilder::new("desktop-settings")
            .window("main")
            .local(false)
            .remote(format!("{}*", origin.as_str()))
            .permission("allow-native-runtime")
            .permission("allow-login-item")
            .permission("allow-set-login-item"),
    )
    .map_err(|e| e.to_string())?;
    *state.origin.lock().unwrap() = Some(origin.origin());
    window
        .navigate(origin.join("/chat").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    while !child.exited() {
        if app.get_webview_window("main").is_none() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    *state.origin.lock().unwrap() = None;
    window.navigate(startup).map_err(|e| e.to_string())?;
    Err("The local Host stopped. Restart to reopen your conversations.".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--background")
                .build(),
        )
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .manage(Desktop::default())
        .invoke_handler(tauri::generate_handler![
            startup_status,
            retry_startup,
            open_diagnostics,
            native::native_runtime,
            native::login_item,
            native::set_login_item
        ])
        .setup(|app| {
            native::tray(app.handle())?;
            let background = std::env::args().any(|a| a == "--background");
            let handle = app.handle().clone();
            let mut builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()));
            if let Some(dir) = std::env::var_os("NOLA_E2E_USER_DATA") {
                builder = builder.data_directory(std::path::PathBuf::from(dir).join("WebView2"));
            }
            let window = builder
                .title("Nola")
                .visible(!background)
                .initialization_script(include_str!("desktop-bridge.js"))
                .inner_size(1160., 780.)
                .min_inner_size(680., 480.)
                .on_navigation(move |url| {
                    local_page(url)
                        || handle
                            .state::<Desktop>()
                            .origin
                            .lock()
                            .is_ok_and(|origin| origin.as_ref() == Some(&url.origin()))
                })
                .on_new_window(|url, _| {
                    if matches!(url.scheme(), "https" | "http")
                        && url.username().is_empty()
                        && url.password().is_none()
                    {
                        let _ = open::that_detached(url.as_str());
                    }
                    NewWindowResponse::Deny
                })
                .build()?;
            let handle = app.handle().clone();
            let startup = if cfg!(windows) {
                "http://tauri.localhost/index.html"
            } else {
                "tauri://localhost/index.html"
            }
            .parse()?;
            thread::spawn(move || {
                if let Err(error) = start_host(&handle, startup) {
                    *handle.state::<Desktop>().failure.lock().unwrap() = Some(error);
                }
            });
            let hide = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hide.hide();
                }
            });
            if !background {
                window.set_focus()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Nola desktop startup failed");
}
