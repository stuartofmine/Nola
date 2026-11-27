// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;
mod native;

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
    thread,
    time::Duration,
};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use url::Url;

struct Desktop {
    origin: Mutex<Option<url::Origin>>,
    failure: Mutex<Option<String>>,
    tray_error: Mutex<Option<native::TrayActionError>>,
    host: Mutex<Option<host::Host>>,
    generation: AtomicU64,
    lifecycle: Mutex<()>,
    theme_preference: Mutex<String>,
    pending_window_state_restore: AtomicBool,
    background_notice_shown: AtomicBool,
}

impl Desktop {
    fn new(background: bool) -> Self {
        Self {
            origin: Mutex::new(None),
            failure: Mutex::new(None),
            tray_error: Mutex::new(None),
            host: Mutex::new(None),
            generation: AtomicU64::new(0),
            lifecycle: Mutex::new(()),
            theme_preference: Mutex::new("system".into()),
            pending_window_state_restore: AtomicBool::new(background),
            background_notice_shown: AtomicBool::new(false),
        }
    }
}

pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn startup_url() -> Url {
    if cfg!(windows) {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    }
    .parse()
    .expect("static startup url")
}

fn startup_url_for_generation(generation: u64) -> Url {
    let mut url = startup_url();
    url.set_query(Some(&format!("gen={generation}")));
    url
}

fn is_allowed_download_url(url: &Url, allowed_origin: Option<&url::Origin>) -> bool {
    let Some(expected) = allowed_origin else {
        return false;
    };
    if &url.origin() == expected {
        return true;
    }
    if url.scheme() == "blob" {
        if let Ok(inner) = Url::parse(url.path()) {
            return &inner.origin() == expected;
        }
    }
    false
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn sanitize_download_filename(raw: &Path) -> String {
    let candidate = raw
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .trim();
    let cleaned: String = candidate
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|ch| ch == '.' || ch == ' ');
    if trimmed.is_empty() {
        "nola-download.bin".into()
    } else {
        let safe = if is_windows_reserved_name(trimmed) {
            format!("_{trimmed}")
        } else {
            trimmed.to_string()
        };
        safe.chars().take(180).collect()
    }
}

fn unique_download_path(dir: &Path, filename: &str) -> PathBuf {
    let direct = dir.join(filename);
    if !direct.exists() {
        return direct;
    }
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("nola-download");
    let ext = path.extension().and_then(|e| e.to_str());
    for index in 1..=999u32 {
        let candidate_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem} ({index}).{ext}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = dir.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.bin", std::process::id()))
}

pub(crate) fn stop_host(app: &AppHandle) {
    let state = app.state::<Desktop>();
    let mut guard = lock_unpoisoned(&state.host);
    if let Some(mut host) = guard.take() {
        host.stop();
    }
}

pub(crate) fn restart_host(app: &AppHandle) {
    let state = app.state::<Desktop>();
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let previous = lock_unpoisoned(&state.host).take();
    *lock_unpoisoned(&state.failure) = None;
    *lock_unpoisoned(&state.origin) = None;
    native::set_tray_tooltip(app, native::label("appName"));
    let startup = startup_url_for_generation(generation);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.navigate(startup.clone());
    }
    let handle = app.clone();
    thread::spawn(move || {
        {
            let state = handle.state::<Desktop>();
            let _guard = lock_unpoisoned(&state.lifecycle);
            if let Some(mut host) = previous {
                host.stop();
            }
        }
        if handle.state::<Desktop>().generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Err(error) = start_host(&handle, startup, generation) {
            let state = handle.state::<Desktop>();
            if state.generation.load(Ordering::SeqCst) == generation {
                *lock_unpoisoned(&state.failure) = Some(error);
                native::set_tray_tooltip(
                    &handle,
                    &format!(
                        "{} — {}",
                        native::label("appName"),
                        native::label("failedTitle")
                    ),
                );
                if let Some(window) = handle.get_webview_window("main") {
                    if !window.is_visible().unwrap_or(false) {
                        native::show(&handle);
                    }
                }
            }
        }
    });
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
    let failure = lock_unpoisoned(&state.failure).clone();
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
    let theme = lock_unpoisoned(&state.theme_preference).clone();
    let tray_error = lock_unpoisoned(&state.tray_error)
        .as_ref()
        .map(native::TrayActionError::to_json);
    Some(
        serde_json::json!({"phase":current,"page":page,"language":native::language(),"failure":failure,"theme":theme,"trayError":tray_error}),
    )
}

#[tauri::command]
fn open_diagnostics(window: tauri::WebviewWindow) -> Result<(), String> {
    if !window.url().is_ok_and(|url| local_page(&url)) {
        return Err(native::label("diagnosticsUnavailable").into());
    }
    let dir = native::desktop_data_dir(window.app_handle())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open::that_detached(dir).map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq, Eq)]
enum NewWindowAction {
    NavigateInApp,
    OpenExternal,
    Ignore,
}

fn classify_new_window(url: &Url, allowed_origin: Option<&url::Origin>) -> NewWindowAction {
    if local_page(url) || allowed_origin == Some(&url.origin()) {
        return NewWindowAction::NavigateInApp;
    }
    if matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
    {
        return NewWindowAction::OpenExternal;
    }
    NewWindowAction::Ignore
}

fn has_background_flag<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().any(|arg| arg.as_ref() == "--background")
}

fn validate_retry_startup(is_local: bool, has_failure: bool) -> Result<(), String> {
    if is_local && has_failure {
        Ok(())
    } else {
        Err(native::label("retryUnavailable").into())
    }
}

#[tauri::command]
fn retry_startup(window: tauri::WebviewWindow) -> Result<(), String> {
    let is_local = window.url().is_ok_and(|url| local_page(&url));
    let has_failure = lock_unpoisoned(&window.state::<Desktop>().failure).is_some();
    validate_retry_startup(is_local, has_failure)?;
    restart_host(window.app_handle());
    Ok(())
}

fn start_host(app: &AppHandle, startup: Url, generation: u64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Window closed")?;
    let path = app.path();
    let data = native::desktop_data_dir(app)?;
    let workspace = native::desktop_workspace_dir(app)?;
    let state = app.state::<Desktop>();
    let origin = {
        let _guard = lock_unpoisoned(&state.lifecycle);
        if state.generation.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        let (mut child, origin) = host::Host::start(
            &path.resource_dir().map_err(|e| e.to_string())?,
            &data,
            &workspace,
        )?;
        if state.generation.load(Ordering::SeqCst) != generation {
            child.stop();
            return Ok(());
        }
        let capability_result = app
            .add_capability(
                tauri::ipc::CapabilityBuilder::new("desktop-settings")
                    .window("main")
                    .local(false)
                    .remote(format!("{}*", origin.as_str()))
                    .permission("allow-native-runtime")
                    .permission("allow-login-item")
                    .permission("allow-set-login-item")
                    .permission("allow-set-theme")
                    .permission("allow-open-logs-folder"),
            )
            .map_err(|e| e.to_string());
        if let Err(err) = capability_result {
            child.stop();
            return Err(err);
        }
        *lock_unpoisoned(&state.host) = Some(child);
        *lock_unpoisoned(&state.origin) = Some(origin.origin());
        origin
    };
    if let Err(err) = origin
        .join("/chat")
        .map_err(|e| e.to_string())
        .and_then(|chat| window.navigate(chat).map_err(|e| e.to_string()))
    {
        if state.generation.load(Ordering::SeqCst) == generation {
            stop_host(app);
            *lock_unpoisoned(&state.origin) = None;
        }
        return Err(err);
    }
    native::set_tray_tooltip(app, native::label("appName"));
    loop {
        if state.generation.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        if app.get_webview_window("main").is_none() {
            stop_host(app);
            return Ok(());
        }
        let exited = match lock_unpoisoned(&state.host).as_mut() {
            Some(host) => host.exited(),
            None => return Ok(()),
        };
        if exited {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if state.generation.load(Ordering::SeqCst) != generation {
        return Ok(());
    }
    let _ = lock_unpoisoned(&state.host).take();
    let message = native::label("hostStopped").to_string();
    *lock_unpoisoned(&state.failure) = Some(message.clone());
    *lock_unpoisoned(&state.origin) = None;
    native::set_tray_tooltip(
        app,
        &format!("{} — {}", native::label("appName"), native::label("failedTitle")),
    );
    window.navigate(startup).map_err(|e| e.to_string())?;
    if !window.is_visible().unwrap_or(false) {
        native::show(app);
    }
    Err(message)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let background = has_background_flag(&args);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !has_background_flag(&args) {
                native::show(app);
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
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
                )
                .skip_initial_state("main")
                .build(),
        )
        .manage(Desktop::new(background))
        .invoke_handler(tauri::generate_handler![
            startup_status,
            retry_startup,
            open_diagnostics,
            native::native_runtime,
            native::login_item,
            native::set_login_item,
            native::set_theme,
            native::open_logs_folder
        ])
        .setup(move |app| {
            native::tray(app.handle())?;
            let nav_handle = app.handle().clone();
            let new_window_handle = app.handle().clone();
            let download_handle = app.handle().clone();
            let (saved_pref, saved_theme) = native::load_persisted_theme(app.handle());
            *lock_unpoisoned(&app.state::<Desktop>().theme_preference) = saved_pref;
            let mut builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()));
            if let Some(dir) = std::env::var_os("NOLA_E2E_USER_DATA") {
                builder = builder.data_directory(PathBuf::from(dir).join("WebView2"));
            }
            if let Some(explicit_theme) = saved_theme {
                builder = builder.theme(Some(explicit_theme));
            }
            let window = builder
                .title(native::label("appName"))
                .visible(false)
                .initialization_script(include_str!("desktop-bridge.js"))
                .inner_size(1160., 780.)
                .min_inner_size(480., 520.)
                .on_navigation(move |url| {
                    if local_page(url)
                        || lock_unpoisoned(&nav_handle.state::<Desktop>().origin).as_ref()
                            == Some(&url.origin())
                    {
                        return true;
                    }
                    if matches!(url.scheme(), "https" | "http")
                        && url.username().is_empty()
                        && url.password().is_none()
                    {
                        let _ = open::that_detached(url.as_str());
                    }
                    false
                })
                .on_new_window(move |url, _| {
                    let action = {
                        let origin =
                            lock_unpoisoned(&new_window_handle.state::<Desktop>().origin).clone();
                        classify_new_window(&url, origin.as_ref())
                    };
                    match action {
                        NewWindowAction::NavigateInApp => {
                            if let Some(window) = new_window_handle.get_webview_window("main") {
                                let _ = window.navigate(url);
                            }
                        }
                        NewWindowAction::OpenExternal => {
                            let _ = open::that_detached(url.as_str());
                        }
                        NewWindowAction::Ignore => {}
                    }
                    NewWindowResponse::Deny
                })
                .on_download(move |_webview, event| match event {
                    DownloadEvent::Requested { url, destination } => {
                        let allowed = is_allowed_download_url(
                            &url,
                            lock_unpoisoned(&download_handle.state::<Desktop>().origin).as_ref(),
                        );
                        if !allowed {
                            return false;
                        }
                        let filename = sanitize_download_filename(destination);
                        let target_dir = download_handle
                            .path()
                            .download_dir()
                            .or_else(|_| native::desktop_data_dir(&download_handle));
                        let Ok(dir) = target_dir else {
                            return false;
                        };
                        if std::fs::create_dir_all(&dir).is_err() {
                            return false;
                        }
                        *destination = unique_download_path(&dir, &filename);
                        true
                    }
                    DownloadEvent::Finished { url, path, success } => {
                        if success
                            && url.path().starts_with("/api/software-update/package")
                        {
                            if let Some(parent) = path.as_deref().and_then(Path::parent) {
                                let _ = open::that_detached(parent);
                            }
                        }
                        true
                    }
                    _ => true,
                })
                .build()?;
            // Apply the persisted/OS theme's canvas color before restoring state or showing to avoid a light flash.
            native::apply_theme_background(&window, saved_theme);
            let handle = app.handle().clone();
            let generation = app
                .state::<Desktop>()
                .generation
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            let startup = startup_url_for_generation(generation);
            thread::spawn(move || {
                if let Err(error) = start_host(&handle, startup, generation) {
                    let state = handle.state::<Desktop>();
                    if state.generation.load(Ordering::SeqCst) == generation {
                        *lock_unpoisoned(&state.failure) = Some(error);
                        native::set_tray_tooltip(
                            &handle,
                            &format!(
                                "{} — {}",
                                native::label("appName"),
                                native::label("failedTitle")
                            ),
                        );
                    }
                }
            });
            let event_window = window.clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = event_window.hide();
                    native::notify_background_hidden_once(event_window.app_handle());
                }
                tauri::WindowEvent::ThemeChanged(theme) => {
                    let follow_system =
                        lock_unpoisoned(&event_window.state::<Desktop>().theme_preference)
                            .as_str()
                            == "system";
                    if follow_system {
                        native::apply_theme_background(&event_window, Some(*theme));
                    }
                }
                _ => {}
            });
            if !background {
                let _ = window.restore_state(
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
                );
                window.show()?;
                window.set_focus()?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Nola desktop startup failed")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop_host(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_url_is_a_recognized_local_page() {
        assert!(local_page(&startup_url()));
        let gen_url = startup_url_for_generation(3);
        assert!(local_page(&gen_url));
        assert_eq!(gen_url.query(), Some("gen=3"));
    }

    #[test]
    fn download_url_validation_allows_only_host_origin_and_host_blobs() {
        let host_origin = Url::parse("http://127.0.0.1:28421").unwrap().origin();
        let pkg = Url::parse("http://127.0.0.1:28421/api/software-update/package?release_id=abc")
            .unwrap();
        let host_blob = Url::parse("blob:http://127.0.0.1:28421/550e8400-e29b-41d4-a716-446655440000")
            .unwrap();
        let other_port = Url::parse("http://127.0.0.1:28422/file.exe").unwrap();
        let external = Url::parse("https://example.com/file.exe").unwrap();

        assert!(is_allowed_download_url(&pkg, Some(&host_origin)));
        assert!(is_allowed_download_url(&host_blob, Some(&host_origin)));
        assert!(!is_allowed_download_url(&other_port, Some(&host_origin)));
        assert!(!is_allowed_download_url(&external, Some(&host_origin)));
        assert!(!is_allowed_download_url(&pkg, None));
    }

    #[test]
    fn download_filename_sanitization_strips_traversal_and_illegal_chars() {
        assert_eq!(
            sanitize_download_filename(Path::new("../../evil:name?.exe")),
            "evil_name_.exe"
        );
        assert_eq!(sanitize_download_filename(Path::new("...")), "nola-download.bin");
        assert_eq!(
            sanitize_download_filename(Path::new("nola-1.0.1-x64.exe")),
            "nola-1.0.1-x64.exe"
        );
        assert_eq!(sanitize_download_filename(Path::new("CON")), "_CON");
        assert_eq!(sanitize_download_filename(Path::new("nul.txt")), "_nul.txt");
        assert_eq!(sanitize_download_filename(Path::new("com1.tar.gz")), "_com1.tar.gz");
        assert_eq!(sanitize_download_filename(Path::new("LPT9")), "_LPT9");
        assert_eq!(sanitize_download_filename(Path::new("console.txt")), "console.txt");
    }

    #[test]
    fn unique_download_path_avoids_overwriting_existing_files() {
        let dir = std::env::temp_dir().join(format!("nola-dl-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_download_path(&dir, "installer.exe");
        assert_eq!(first, dir.join("installer.exe"));
        std::fs::write(&first, b"existing").unwrap();
        let second = unique_download_path(&dir, "installer.exe");
        assert_eq!(second, dir.join("installer (1).exe"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retry_startup_validation_rejects_missing_preconditions() {
        assert!(validate_retry_startup(true, true).is_ok());
        assert!(validate_retry_startup(false, true).is_err());
        assert!(validate_retry_startup(true, false).is_err());
        assert!(validate_retry_startup(false, false).is_err());
    }

    #[test]
    fn classify_new_window_keeps_same_origin_and_local_in_app() {
        let host_origin = Url::parse("http://127.0.0.1:28421").unwrap().origin();
        let same_origin = Url::parse("http://127.0.0.1:28421/settings").unwrap();
        let external = Url::parse("https://example.com/docs").unwrap();
        let credentialed = Url::parse("https://user:pass@example.com/docs").unwrap();
        let file_url = Url::parse("file:///C:/Windows/win.ini").unwrap();

        assert_eq!(
            classify_new_window(&same_origin, Some(&host_origin)),
            NewWindowAction::NavigateInApp
        );
        assert_eq!(
            classify_new_window(&startup_url(), Some(&host_origin)),
            NewWindowAction::NavigateInApp
        );
        assert_eq!(
            classify_new_window(&external, Some(&host_origin)),
            NewWindowAction::OpenExternal
        );
        assert_eq!(
            classify_new_window(&credentialed, Some(&host_origin)),
            NewWindowAction::Ignore
        );
        assert_eq!(
            classify_new_window(&file_url, Some(&host_origin)),
            NewWindowAction::Ignore
        );
    }

    #[test]
    fn background_flag_detection_handles_second_instance_args() {
        assert!(has_background_flag(&["nola-desktop.exe", "--background"]));
        assert!(!has_background_flag(&["nola-desktop.exe"]));
        assert!(!has_background_flag(&["nola-desktop.exe", "--background=false"]));
    }
}
