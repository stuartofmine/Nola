// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;

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
fn startup_failure(window: tauri::WebviewWindow) -> Option<String> {
    if !local_page(&window.url().ok()?) {
        return None;
    }
    window.state::<Desktop>().failure.lock().ok()?.clone()
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
    let workspace = path
        .document_dir()
        .map_err(|e| e.to_string())?
        .join("Nola Workspace");
    let (mut child, origin) = host::Host::start(
        &path.resource_dir().map_err(|e| e.to_string())?,
        &path.app_local_data_dir().map_err(|e| e.to_string())?,
        &workspace,
    )?;
    let state = app.state::<Desktop>();
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
        .manage(Desktop::default())
        .invoke_handler(tauri::generate_handler![startup_failure, retry_startup])
        .setup(|app| {
            let handle = app.handle().clone();
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Nola")
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
            window.set_focus()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Nola desktop startup failed");
}
