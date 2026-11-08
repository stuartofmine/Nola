// SPDX-License-Identifier: MIT
// Shared llama.cpp model catalog and lifecycle for CLI and Web.

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::store::NolaStore;
use nola_application::{ModelProfile, ModelRuntime, ModelRuntimePhase};

pub const LOCAL_CONTEXT_TOKENS: usize = 65_536;

#[derive(Debug, Clone, Copy)]
pub struct LocalModelProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub subtitle: &'static str,
    pub repository: &'static str,
    pub filename: &'static str,
    pub download_url: &'static str,
    pub download_bytes: u64,
    pub estimated_runtime_bytes: u64,
    pub recommended_memory_bytes: u64,
    pub native_context_tokens: usize,
    pub tier: &'static str,
}

pub const LOCAL_MODELS: [LocalModelProfile; 2] = [
    LocalModelProfile {
        id: "qwen3.5-0.8b-q4",
        name: "Qwen3.5 0.8B",
        subtitle: "Fast and compact for lightweight daily tasks",
        repository: "bartowski/Qwen_Qwen3.5-0.8B-GGUF",
        filename: "Qwen_Qwen3.5-0.8B-Q4_K_M.gguf",
        download_url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-0.8B-GGUF/resolve/main/Qwen_Qwen3.5-0.8B-Q4_K_M.gguf",
        download_bytes: 579_615_840,
        estimated_runtime_bytes: 3_000_000_000,
        recommended_memory_bytes: 8_000_000_000,
        native_context_tokens: 262_144,
        tier: "compact",
    },
    LocalModelProfile {
        id: "qwen3.5-4b-q4km",
        name: "Qwen3.5 4B",
        subtitle: "Recommended balance for chat, writing, and agent work",
        repository: "bartowski/Qwen_Qwen3.5-4B-GGUF",
        filename: "Qwen_Qwen3.5-4B-Q4_K_M.gguf",
        download_url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q4_K_M.gguf",
        download_bytes: 3_013_027_808,
        estimated_runtime_bytes: 7_000_000_000,
        recommended_memory_bytes: 16_000_000_000,
        native_context_tokens: 262_144,
        tier: "recommended",
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadState {
    status: String,
    downloaded_bytes: u64,
    total_bytes: u64,
    error: Option<String>,
}

impl DownloadState {
    fn idle(total_bytes: u64) -> Self {
        Self {
            status: "idle".to_string(),
            downloaded_bytes: 0,
            total_bytes,
            error: None,
        }
    }
}

struct RuntimeState {
    child: Child,
    launch: LocalModelLaunchSnapshot,
    instance_id: String,
    port: u16,
    api_key: String,
}

/// Facts used to launch an engine, independent of mutable profile revisions.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalModelLaunchSnapshot {
    pub model_id: String,
    pub binary_path: PathBuf,
    pub artifact_path: PathBuf,
    pub server_subcommand: bool,
    pub parameters: LocalModelParameters,
}

/// Host-private live observation; it is not a persisted availability guarantee.
#[derive(Debug, Clone)]
pub struct LocalModelLiveSnapshot {
    pub launch: LocalModelLaunchSnapshot,
    pub endpoint: LocalModelEndpoint,
}

fn process_is_running(status: std::io::Result<Option<std::process::ExitStatus>>) -> Result<bool, String> {
    status.map(|status| status.is_none()).map_err(|error| format!("Cannot inspect local model process: {error}"))
}

impl RuntimeState {
    fn can_reuse(&mut self, launch: &LocalModelLaunchSnapshot) -> Result<bool, String> {
        Ok(process_is_running(self.child.try_wait())? && self.launch == *launch)
    }

    fn live_snapshot(&mut self, model_id: &str) -> Result<Option<LocalModelLiveSnapshot>, String> {
        if self.launch.model_id != model_id || !process_is_running(self.child.try_wait())? {
            return Ok(None);
        }
        Ok(Some(LocalModelLiveSnapshot { launch: self.launch.clone(), endpoint: endpoint_from_state(self) }))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelEndpoint {
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
    pub context_window_tokens: usize,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalModelParameters {
    pub context_tokens: usize,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub min_p: f64,
    pub batch_size: usize,
    pub threads: usize,
    pub gpu_layers: i32,
    pub flash_attention: bool,
}

impl Default for LocalModelParameters {
    fn default() -> Self {
        Self {
            context_tokens: LOCAL_CONTEXT_TOKENS,
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            min_p: 0.05,
            batch_size: 512,
            threads: 0,
            gpu_layers: 999,
            flash_attention: true,
        }
    }
}

impl LocalModelParameters {
    pub fn validated(mut self, model: &LocalModelProfile) -> Result<Self, String> {
        if self.context_tokens < LOCAL_CONTEXT_TOKENS {
            return Err(format!(
                "contextTokens must be at least {LOCAL_CONTEXT_TOKENS}"
            ));
        }
        self.context_tokens = self.context_tokens.min(model.native_context_tokens);
        self.temperature = self.temperature.clamp(0.0, 2.0);
        self.top_p = self.top_p.clamp(0.0, 1.0);
        self.top_k = self.top_k.clamp(1, 200);
        self.min_p = self.min_p.clamp(0.0, 1.0);
        self.batch_size = self.batch_size.clamp(32, 4096);
        self.threads = self.threads.min(256);
        self.gpu_layers = self.gpu_layers.clamp(0, 999);
        Ok(self)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LocalModelPreferences {
    selected_model_id: Option<String>,
    parameters: HashMap<String, LocalModelParameters>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct InstalledModelCatalog {
    models: Vec<InstalledModelRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct InstalledModelRecord {
    id: String,
    name: String,
    repository: String,
    revision: Option<String>,
    runtime: String,
    format: String,
    quantization: Option<String>,
    context_window_tokens: usize,
    artifact_path: String,
    artifact_bytes: u64,
    estimated_peak_memory_bytes: Option<u64>,
    recommended_memory_bytes: Option<u64>,
    state: String,
    enabled: bool,
}

static DOWNLOADS: OnceLock<Mutex<HashMap<String, DownloadState>>> = OnceLock::new();
static RUNTIME: OnceLock<Mutex<Option<RuntimeState>>> = OnceLock::new();

fn downloads() -> &'static Mutex<HashMap<String, DownloadState>> {
    DOWNLOADS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn runtime() -> &'static Mutex<Option<RuntimeState>> {
    RUNTIME.get_or_init(|| Mutex::new(None))
}

pub fn profile(model_id: &str) -> Option<&'static LocalModelProfile> {
    let value = model_id.trim();
    LOCAL_MODELS.iter().find(|model| {
        model.id.eq_ignore_ascii_case(value)
            || model.name.eq_ignore_ascii_case(value)
            || model.repository.eq_ignore_ascii_case(value)
    })
}

pub fn model_ids() -> Vec<String> {
    let mut ids = LOCAL_MODELS
        .iter()
        .map(|model| model.id.to_string())
        .collect::<Vec<_>>();
    for model in load_installed_catalog().models {
        if model.enabled
            && model.state == "configured"
            && !ids.iter().any(|id| id.eq_ignore_ascii_case(&model.id))
        {
            ids.push(model.id);
        }
    }
    ids
}

pub fn contains_model(model_id: &str) -> bool {
    profile(model_id).is_some() || installed_record(model_id).is_some()
}

pub fn models_root() -> Result<PathBuf, String> {
    let home = std::env::var_os("NOLA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".nola")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".nola")))
        .ok_or_else(|| "Cannot resolve the Nola home directory".to_string())?;
    Ok(home.join("models").join("llama.cpp"))
}

fn installed_catalog_path() -> Result<PathBuf, String> {
    Ok(models_root()?.join("catalog.json"))
}

fn load_installed_catalog() -> InstalledModelCatalog {
    installed_catalog_path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save_installed_catalog(value: &InstalledModelCatalog) -> Result<(), String> {
    let path = installed_catalog_path()?;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "Invalid model catalog path".to_string())?,
    )
    .map_err(|error| format!("Failed to create model catalog directory: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value)
            .map_err(|error| format!("Failed to encode model catalog: {error}"))?,
    )
    .map_err(|error| format!("Failed to save model catalog: {error}"))?;
    fs::rename(temporary, path).map_err(|error| format!("Failed to commit model catalog: {error}"))
}

fn installed_record(model_id: &str) -> Option<InstalledModelRecord> {
    let value = model_id.trim();
    load_installed_catalog().models.into_iter().find(|model| {
        model.enabled
            && model.state == "configured"
            && (model.id.eq_ignore_ascii_case(value)
                || model.name.eq_ignore_ascii_case(value)
                || model.repository.eq_ignore_ascii_case(value))
    })
}

fn validated_installed_path(model: &InstalledModelRecord) -> Result<PathBuf, String> {
    if model.runtime != "llama.cpp" || model.format != "gguf" {
        return Err(format!(
            "{} is downloaded, but its {} / {} runtime is not supported by this host",
            model.name, model.runtime, model.format
        ));
    }
    let root = models_root()?
        .canonicalize()
        .map_err(|error| format!("Failed to resolve models root: {error}"))?;
    let path = PathBuf::from(&model.artifact_path)
        .canonicalize()
        .map_err(|error| format!("Installed model artifact is unavailable: {error}"))?;
    if !path.starts_with(&root) {
        return Err("Installed model artifact is outside the Nola models root".to_string());
    }
    let size = fs::metadata(&path)
        .map_err(|error| format!("Installed model artifact is unavailable: {error}"))?
        .len();
    if size != model.artifact_bytes {
        return Err(format!(
            "Installed model artifact size changed: {size} != {}",
            model.artifact_bytes
        ));
    }
    Ok(path)
}

pub fn model_path(model: &LocalModelProfile) -> Result<PathBuf, String> {
    Ok(models_root()?.join(model.id).join(model.filename))
}

fn preferences_path() -> Result<PathBuf, String> {
    Ok(models_root()?.join("settings.json"))
}

fn load_preferences() -> LocalModelPreferences {
    preferences_path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn selected_model_id() -> Option<String> {
    NolaStore::open_default()
        .and_then(|store| store.model_profiles())
        .ok()
        .and_then(|profiles| {
            profiles
                .into_iter()
                .find(|profile| profile.provider == "local" && profile.selected && profile.enabled)
                .map(|profile| profile.id)
        })
        .or_else(|| load_preferences().selected_model_id)
}

pub fn select_model(model_id: &str) -> Result<(), String> {
    let resolved_id = if let Some(model) = profile(model_id) {
        model.id.to_string()
    } else if let Some(model) = installed_record(model_id) {
        validated_installed_path(&model)?;
        model.id
    } else {
        return Err(format!("Unknown local model: {model_id}"));
    };
    let parameters = parameters(&resolved_id)?;
    let store = NolaStore::open_default()?;
    let existing = store.model_profile(&resolved_id)?;
    store.upsert_model_profile(stored_model_profile(
        &resolved_id,
        parameters,
        true,
        existing.as_ref(),
    )?)
}

pub fn parameters(model_id: &str) -> Result<LocalModelParameters, String> {
    let canonical = NolaStore::open_default()
        .and_then(|store| store.model_profile(model_id))
        .ok()
        .flatten()
        .and_then(|profile| {
            serde_json::from_str::<LocalModelParameters>(&profile.parameters_json).ok()
        });
    if let Some(model) = profile(model_id) {
        return canonical
            .or_else(|| load_preferences().parameters.get(model.id).cloned())
            .unwrap_or_default()
            .validated(model);
    }
    let model =
        installed_record(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    let mut value = canonical
        .or_else(|| load_preferences().parameters.get(&model.id).cloned())
        .unwrap_or_default();
    if value.context_tokens < LOCAL_CONTEXT_TOKENS {
        return Err(format!(
            "contextTokens must be at least {LOCAL_CONTEXT_TOKENS}"
        ));
    }
    value.context_tokens = value
        .context_tokens
        .min(model.context_window_tokens.max(LOCAL_CONTEXT_TOKENS));
    value.temperature = value.temperature.clamp(0.0, 2.0);
    value.top_p = value.top_p.clamp(0.0, 1.0);
    value.top_k = value.top_k.clamp(1, 200);
    value.min_p = value.min_p.clamp(0.0, 1.0);
    value.batch_size = value.batch_size.clamp(32, 4096);
    value.threads = value.threads.min(256);
    value.gpu_layers = value.gpu_layers.clamp(0, 999);
    Ok(value)
}

pub fn save_parameters(
    model_id: &str,
    value: LocalModelParameters,
) -> Result<LocalModelParameters, String> {
    let resolved_id = profile(model_id)
        .map(|model| model.id.to_string())
        .or_else(|| installed_record(model_id).map(|model| model.id))
        .ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    let revision = NolaStore::open_default()?
        .model_profile(&resolved_id)?
        .map_or(0, |profile| profile.revision);
    save_parameters_at_revision(
        &resolved_id,
        value,
        u64::try_from(revision).map_err(|_| "invalid model revision".to_string())?,
    )
}

pub fn save_parameters_at_revision(
    model_id: &str,
    value: LocalModelParameters,
    expected_revision: u64,
) -> Result<LocalModelParameters, String> {
    let built_in = profile(model_id);
    let installed = if built_in.is_none() {
        installed_record(model_id)
    } else {
        None
    };
    let resolved_id = built_in
        .map(|model| model.id.to_string())
        .or_else(|| installed.as_ref().map(|model| model.id.clone()))
        .ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    let native_context = built_in
        .map(|model| model.native_context_tokens)
        .or_else(|| installed.as_ref().map(|model| model.context_window_tokens))
        .unwrap_or(LOCAL_CONTEXT_TOKENS)
        .max(LOCAL_CONTEXT_TOKENS);
    let mut value = value;
    if value.context_tokens < LOCAL_CONTEXT_TOKENS {
        return Err(format!(
            "contextTokens must be at least {LOCAL_CONTEXT_TOKENS}"
        ));
    }
    value.context_tokens = value.context_tokens.min(native_context);
    value.temperature = value.temperature.clamp(0.0, 2.0);
    value.top_p = value.top_p.clamp(0.0, 1.0);
    value.top_k = value.top_k.clamp(1, 200);
    value.min_p = value.min_p.clamp(0.0, 1.0);
    value.batch_size = value.batch_size.clamp(32, 4096);
    value.threads = value.threads.min(256);
    value.gpu_layers = value.gpu_layers.clamp(0, 999);
    let store = NolaStore::open_default()?;
    store.configure_model_parameters(
        stored_model_profile(&resolved_id, value.clone(), false, None)?,
        expected_revision,
    )?;
    Ok(value)
}

fn stored_model_profile(
    model_id: &str,
    parameters: LocalModelParameters,
    selected: bool,
    existing: Option<&ModelProfile>,
) -> Result<ModelProfile, String> {
    let (display_name, model_name, source_json) = if let Some(profile) = profile(model_id) {
        (
            profile.name.to_string(),
            profile.id.to_string(),
            serde_json::to_string(&json!({
                "repository": profile.repository,
                "filename": profile.filename,
                "downloadUrl": profile.download_url,
                "artifactPath": model_path(profile)?.display().to_string(),
            }))
            .map_err(|error| format!("Failed to encode local model source: {error}"))?,
        )
    } else {
        let profile =
            installed_record(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
        (
            profile.name,
            profile.id,
            serde_json::to_string(&json!({
                "repository": profile.repository,
                "revision": profile.revision,
                "artifactPath": profile.artifact_path,
                "format": profile.format,
                "quantization": profile.quantization,
            }))
            .map_err(|error| format!("Failed to encode local model source: {error}"))?,
        )
    };
    let now = unix_epoch_millis();
    Ok(ModelProfile {
        id: model_id.to_string(),
        display_name,
        provider: "local".to_string(),
        model_name,
        runtime: "llama.cpp".to_string(),
        source_json,
        parameters_json: serde_json::to_string(&parameters)
            .map_err(|error| format!("Failed to encode local model parameters: {error}"))?,
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: existing.map_or(now, |profile| profile.created_at_ms),
        updated_at_ms: now,
        deleted_at_ms: None,
    })
}

fn unix_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn report_runtime(
    profile_id: &str,
    phase: &str,
    endpoint: Option<&LocalModelEndpoint>,
    process_id: Option<u32>,
    error: Option<String>,
) {
    let now = unix_epoch_millis();
    let endpoint_json = endpoint.and_then(|value| serde_json::to_string(value).ok());
    let _ = NolaStore::open_default().and_then(|store| {
        let phase = ModelRuntimePhase::try_from(phase)?;
        store.report_model_runtime(ModelRuntime {
            host_id: "cli-web:local".to_string(),
            profile_id: profile_id.to_string(),
            phase,
            endpoint_json,
            process_id: process_id.map(i64::from),
            error,
            started_at_ms: matches!(
                phase,
                ModelRuntimePhase::Starting | ModelRuntimePhase::Ready
            )
            .then_some(now),
            stopped_at_ms: (phase == ModelRuntimePhase::Stopped).then_some(now),
            updated_at_ms: now,
        })
    });
}

pub fn is_installed(model: &LocalModelProfile) -> bool {
    model_path(model)
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len() >= model.download_bytes)
        .unwrap_or(false)
}

pub fn llama_server_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("NOLA_LLAMA_SERVER") {
        let path = PathBuf::from(explicit);
        if executable_file(&path) {
            return Some(path);
        }
    }
    let candidates: &[&str] = if cfg!(windows) {
        &["llama-server.exe", "llama.exe"]
    } else {
        &["llama-server", "llama"]
    };
    if let Some(path_value) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path_value) {
            for candidate in candidates {
                let path = directory.join(candidate);
                if executable_file(&path) {
                    return Some(path);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    for path in [
        PathBuf::from("/opt/homebrew/bin/llama-server"),
        PathBuf::from("/usr/local/bin/llama-server"),
        PathBuf::from("/opt/homebrew/bin/llama"),
        PathBuf::from("/usr/local/bin/llama"),
    ] {
        if executable_file(&path) {
            return Some(path);
        }
    }
    None
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn physical_memory_bytes() -> Option<u64> {
    #[cfg(unix)]
    unsafe {
        let pages = libc::sysconf(libc::_SC_PHYS_PAGES);
        let page_size = libc::sysconf(libc::_SC_PAGE_SIZE);
        if pages > 0 && page_size > 0 {
            return u64::try_from(pages).ok().and_then(|count| {
                u64::try_from(page_size)
                    .ok()
                    .map(|size| count.saturating_mul(size))
            });
        }
    }
    None
}

pub fn recommended_model_id() -> &'static str {
    if physical_memory_bytes().unwrap_or(0) >= LOCAL_MODELS[1].recommended_memory_bytes {
        LOCAL_MODELS[1].id
    } else {
        LOCAL_MODELS[0].id
    }
}

pub fn catalog_json() -> Value {
    let memory = physical_memory_bytes();
    let binary = llama_server_path();
    let running_id = running_model_id();
    let states = downloads().lock().ok();
    let preferences = load_preferences();
    let mut disk_usage_bytes = LOCAL_MODELS.iter().filter_map(|model| {
        model_path(model).ok().and_then(|path| fs::metadata(path).ok()).map(|metadata| metadata.len())
    }).fold(0u64, u64::saturating_add);
    // Parameters and their revision must come from the same canonical read.
    let canonical = NolaStore::open_default().and_then(|store| store.model_profiles());
    let configuration_error = canonical.as_ref().err().cloned();
    let canonical: HashMap<String, ModelProfile> = canonical
        .unwrap_or_default()
        .into_iter()
        .map(|profile| (profile.id.clone(), profile))
        .collect();
    let selected_id = canonical.values().find(|profile| {
        profile.provider == "local" && profile.selected && profile.enabled && profile.deleted_at_ms.is_none()
    }).map(|profile| profile.id.as_str()).or(preferences.selected_model_id.as_deref());
    let mut models = LOCAL_MODELS
        .iter()
        .map(|model| {
            let state = states
                .as_ref()
                .and_then(|values| values.get(model.id).cloned())
                .unwrap_or_else(|| DownloadState::idle(model.download_bytes));
            json!({
                "id": model.id,
                "name": model.name,
                "subtitle": model.subtitle,
                "repository": model.repository,
                "quantization": "Q4_K_M",
                "tier": model.tier,
                "downloadBytes": model.download_bytes,
                "estimatedRuntimeBytes": model.estimated_runtime_bytes,
                "recommendedMemoryBytes": model.recommended_memory_bytes,
                "contextWindowTokens": LOCAL_CONTEXT_TOKENS,
                "nativeContextTokens": model.native_context_tokens,
                "installed": is_installed(model),
                "running": running_id.as_deref() == Some(model.id),
                "selected": selected_id == Some(model.id),
                "deviceCompatible": memory.map(|value| value >= model.recommended_memory_bytes),
                "download": state,
                "parameters": canonical.get(model.id).and_then(|profile| serde_json::from_str::<Value>(&profile.parameters_json).ok()).unwrap_or_else(|| json!(LocalModelParameters::default())),
                "configurationRevision": if configuration_error.is_none() { canonical.get(model.id).map_or(Some(0), |profile| u64::try_from(profile.revision).ok()) } else { None },
            })
        })
        .collect::<Vec<_>>();
    for model in load_installed_catalog().models {
        if LOCAL_MODELS
            .iter()
            .any(|built_in| built_in.id.eq_ignore_ascii_case(&model.id))
        {
            continue;
        }
        let path_result = validated_installed_path(&model);
        let runtime_supported = model.runtime == "llama.cpp" && model.format == "gguf";
        let installed = path_result.is_ok();
        if installed { disk_usage_bytes = disk_usage_bytes.saturating_add(model.artifact_bytes); }
        models.push(json!({
            "id": model.id,
            "name": model.name,
            "subtitle": if runtime_supported { "Installed from an immutable model snapshot" } else { "Downloaded; runtime unavailable on this host" },
            "repository": model.repository,
            "revision": model.revision,
            "quantization": model.quantization,
            "tier": "installed",
            "downloadBytes": model.artifact_bytes,
            "estimatedRuntimeBytes": model.estimated_peak_memory_bytes,
            "recommendedMemoryBytes": model.recommended_memory_bytes.or(model.estimated_peak_memory_bytes),
            "contextWindowTokens": model.context_window_tokens.max(LOCAL_CONTEXT_TOKENS),
            "nativeContextTokens": model.context_window_tokens.max(LOCAL_CONTEXT_TOKENS),
            "installed": installed,
            "running": running_id.as_deref() == Some(model.id.as_str()),
            "selected": selected_id == Some(model.id.as_str()),
            "deviceCompatible": runtime_supported,
            "runtimeSupported": runtime_supported,
            "runtimeIssue": path_result.err(),
            "download": DownloadState { status: if installed { "installed".to_string() } else { "invalid".to_string() }, downloaded_bytes: if installed { model.artifact_bytes } else { 0 }, total_bytes: model.artifact_bytes, error: None },
            "parameters": canonical.get(&model.id).and_then(|profile| serde_json::from_str::<Value>(&profile.parameters_json).ok()).unwrap_or_else(|| json!(LocalModelParameters::default())),
            "configurationRevision": if configuration_error.is_none() { canonical.get(&model.id).map_or(Some(0), |profile| u64::try_from(profile.revision).ok()) } else { None },
        }));
    }
    json!({
        "backend": "llama.cpp",
        "configurationError": configuration_error,
        "binaryAvailable": binary.is_some(),
        "binaryPath": binary.map(|path| path.display().to_string()),
        "installHint": llama_install_hint(),
        "modelsRoot": models_root().ok().map(|path| path.display().to_string()),
        "diskUsageBytes": disk_usage_bytes,
        "physicalMemoryBytes": memory,
        "recommendedModelId": recommended_model_id(),
        "minimumContextWindowTokens": LOCAL_CONTEXT_TOKENS,
        "models": models,
    })
}

pub fn start_download(model_id: &str) -> Result<(), String> {
    let model = *profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    if is_installed(&model) {
        return Ok(());
    }
    {
        let mut states = downloads()
            .lock()
            .map_err(|_| "Local model download state is unavailable".to_string())?;
        if states
            .get(model.id)
            .map(|state| state.status == "downloading")
            .unwrap_or(false)
        {
            return Ok(());
        }
        states.insert(
            model.id.to_string(),
            DownloadState {
                status: "downloading".to_string(),
                downloaded_bytes: partial_download_size(&model),
                total_bytes: model.download_bytes,
                error: None,
            },
        );
    }
    thread::Builder::new()
        .name(format!("nola-model-download-{}", model.id))
        .spawn(move || {
            let result = download_model_inner(&model, |downloaded, total| {
                if let Ok(mut states) = downloads().lock() {
                    states.insert(
                        model.id.to_string(),
                        DownloadState {
                            status: "downloading".to_string(),
                            downloaded_bytes: downloaded,
                            total_bytes: total,
                            error: None,
                        },
                    );
                }
            });
            if let Ok(mut states) = downloads().lock() {
                let (status, downloaded_bytes, error) = match result {
                    Ok(()) => ("installed", model.download_bytes, None),
                    Err(error) => ("failed", partial_download_size(&model), Some(error)),
                };
                states.insert(
                    model.id.to_string(),
                    DownloadState {
                        status: status.to_string(),
                        downloaded_bytes,
                        total_bytes: model.download_bytes,
                        error,
                    },
                );
            }
        })
        .map_err(|error| format!("Failed to start model download: {error}"))?;
    Ok(())
}

pub fn download_model<F>(model_id: &str, progress: F) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let model = profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    download_model_inner(model, progress)
}

fn download_model_inner<F>(model: &LocalModelProfile, mut progress: F) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let destination = model_path(model)?;
    if is_installed(model) {
        progress(model.download_bytes, model.download_bytes);
        return Ok(());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Invalid local model destination".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create model directory: {error}"))?;
    let partial = destination.with_extension("gguf.part");
    let existing = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 60 * 8))
        .build()
        .map_err(|error| format!("Failed to initialize model downloader: {error}"))?;
    let mut request = client.get(model.download_url);
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let mut response = request
        .send()
        .map_err(|error| format!("Model download failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Model download failed with HTTP {}",
            response.status()
        ));
    }
    let resumed = response.status().as_u16() == 206;
    let mut downloaded = if resumed { existing } else { 0 };
    let response_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| model.download_bytes.saturating_sub(downloaded));
    let total = if resumed {
        downloaded.saturating_add(response_length)
    } else {
        response_length.max(model.download_bytes)
    };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&partial)
        .map_err(|error| format!("Failed to open model download: {error}"))?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    progress(downloaded, total);
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| format!("Failed while downloading model: {error}"))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|error| format!("Failed to save model: {error}"))?;
        downloaded = downloaded.saturating_add(count as u64);
        progress(downloaded, total);
    }
    file.flush()
        .map_err(|error| format!("Failed to finish model download: {error}"))?;
    if downloaded < model.download_bytes {
        return Err(format!(
            "Model download is incomplete: {downloaded} of {} bytes",
            model.download_bytes
        ));
    }
    fs::rename(&partial, &destination)
        .map_err(|error| format!("Failed to install downloaded model: {error}"))?;
    Ok(())
}

fn partial_download_size(model: &LocalModelProfile) -> u64 {
    model_path(model)
        .ok()
        .map(|path| path.with_extension("gguf.part"))
        .and_then(|path| fs::metadata(path).ok())
        .map(|value| value.len())
        .unwrap_or(0)
}

pub fn start_model(model_id: &str) -> Result<LocalModelEndpoint, String> {
    let built_in = profile(model_id);
    let installed = if built_in.is_none() {
        installed_record(model_id)
    } else {
        None
    };
    let resolved_id = built_in
        .map(|model| model.id.to_string())
        .or_else(|| installed.as_ref().map(|model| model.id.clone()))
        .ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    let resolved_path = if let Some(model) = built_in {
        if !is_installed(model) {
            return Err(format!("{} is not installed", model.name));
        }
        model_path(model)?
    } else {
        validated_installed_path(installed.as_ref().expect("installed model resolved"))?
    };
    let binary = llama_server_path()
        .ok_or_else(|| format!("llama.cpp is not installed. {}", llama_install_hint()))?;
    let parameters = parameters(&resolved_id)?;
    let launch = LocalModelLaunchSnapshot {
        model_id: resolved_id.clone(),
        binary_path: binary.canonicalize().map_err(|error| format!("Cannot resolve engine executable: {error}"))?,
        artifact_path: resolved_path.canonicalize().map_err(|error| format!("Cannot resolve model artifact: {error}"))?,
        server_subcommand: binary.file_stem().and_then(|value| value.to_str()) == Some("llama"),
        parameters: parameters.clone(),
    };
    if let Ok(store) = NolaStore::open_default() {
        if store.model_profile(&resolved_id)?.is_none() {
            store.upsert_model_profile(stored_model_profile(
                &resolved_id,
                parameters.clone(),
                false,
                None,
            )?)?;
        }
    }
    let mut guard = runtime()
        .lock()
        .map_err(|_| "Local model runtime is unavailable".to_string())?;
    if let Some(state) = guard.as_mut() {
        if state.can_reuse(&launch)? {
            return Ok(endpoint_from_state(state));
        }
        let _ = state.child.kill();
        let _ = state.child.wait();
        *guard = None;
    }

    let port = available_loopback_port()?;
    let api_key = Uuid::new_v4().simple().to_string();
    let mut command = Command::new(&launch.binary_path);
    if launch.server_subcommand {
        command.arg("serve");
    }
    command
        .arg("--model")
        .arg(&launch.artifact_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg(parameters.context_tokens.to_string())
        .arg("--parallel")
        .arg("1")
        .arg("--batch-size")
        .arg(parameters.batch_size.to_string())
        .arg("--n-gpu-layers")
        .arg(parameters.gpu_layers.to_string())
        .arg("--temp")
        .arg(parameters.temperature.to_string())
        .arg("--top-p")
        .arg(parameters.top_p.to_string())
        .arg("--top-k")
        .arg(parameters.top_k.to_string())
        .arg("--min-p")
        .arg(parameters.min_p.to_string())
        .arg("--jinja")
        .arg("--api-key")
        .arg(&api_key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if parameters.threads > 0 {
        command.arg("--threads").arg(parameters.threads.to_string());
    }
    if parameters.flash_attention {
        command.arg("--flash-attn").arg("on");
    }
    report_runtime(&resolved_id, "starting", None, None, None);
    let mut child = command.spawn().map_err(|error| {
        let message = format!(
            "Failed to launch llama.cpp at {}: {error}",
            binary.display()
        );
        report_runtime(&resolved_id, "failed", None, None, Some(message.clone()));
        message
    })?;
    let address = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|error| format!("Invalid local model address: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok() {
            break;
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Failed to inspect llama.cpp: {error}"))?
        {
            return Err(format!("llama.cpp exited before becoming ready: {status}"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("llama.cpp did not become ready within 30 seconds".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let state = RuntimeState {
        child,
        launch,
        instance_id: Uuid::new_v4().to_string(),
        port,
        api_key,
    };
    let endpoint = endpoint_from_state(&state);
    report_runtime(
        &endpoint.model_id,
        "ready",
        Some(&endpoint),
        Some(state.child.id()),
        None,
    );
    *guard = Some(state);
    Ok(endpoint)
}

fn endpoint_from_state(state: &RuntimeState) -> LocalModelEndpoint {
    LocalModelEndpoint {
        base_url: format!("http://127.0.0.1:{}/v1", state.port),
        api_key: state.api_key.clone(),
        model_id: state.launch.model_id.clone(),
        context_window_tokens: state.launch.parameters.context_tokens,
        instance_id: state.instance_id.clone(),
    }
}

pub fn running_endpoint(model_id: &str) -> Option<LocalModelEndpoint> {
    running_snapshot(model_id).ok().flatten().map(|snapshot| snapshot.endpoint)
}

pub fn running_snapshot(model_id: &str) -> Result<Option<LocalModelLiveSnapshot>, String> {
    let mut guard = runtime().lock().map_err(|_| "Local model runtime is unavailable")?;
    match guard.as_mut() {
        Some(state) => state.live_snapshot(model_id),
        None => Ok(None),
    }
}

fn available_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Failed to reserve a local model port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("Failed to read local model port: {error}"))
}

pub fn running_model_id() -> Option<String> {
    let mut guard = runtime().lock().ok()?;
    let state = guard.as_mut()?;
    match process_is_running(state.child.try_wait()) {
        Ok(true) => Some(state.launch.model_id.clone()),
        Ok(false) => {
            *guard = None;
            None
        }
        Err(_) => None,
    }
}

pub fn remove_model(model_id: &str) -> Result<(), String> {
    if profile(model_id).is_none() {
        let model =
            installed_record(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
        if running_model_id().as_deref() == Some(model.id.as_str()) {
            shutdown();
        }
        let root = models_root()?
            .canonicalize()
            .map_err(|error| format!("Failed to resolve models root: {error}"))?;
        let artifact = validated_installed_path(&model)?;
        let model_directory = artifact
            .ancestors()
            .find(|path| path.parent() == Some(root.as_path()))
            .ok_or_else(|| "Installed model directory is outside the models root".to_string())?;
        fs::remove_dir_all(model_directory)
            .map_err(|error| format!("Failed to remove local model: {error}"))?;
        let mut catalog = load_installed_catalog();
        catalog.models.retain(|item| item.id != model.id);
        save_installed_catalog(&catalog)?;
        NolaStore::open_default()?.delete_model_profile(&model.id, unix_epoch_millis())?;
        return Ok(());
    }
    let model = profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    if running_model_id().as_deref() == Some(model.id) {
        shutdown();
    }
    let directory = model_path(model)?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Invalid local model directory".to_string())?;
    if directory.exists() {
        fs::remove_dir_all(directory)
            .map_err(|error| format!("Failed to remove local model: {error}"))?;
    }
    if let Ok(mut states) = downloads().lock() {
        states.remove(model.id);
    }
    NolaStore::open_default()?.delete_model_profile(model.id, unix_epoch_millis())?;
    Ok(())
}

pub fn shutdown() {
    if let Ok(mut guard) = runtime().lock() {
        if let Some(mut state) = guard.take() {
            let model_id = state.launch.model_id.clone();
            let _ = state.child.kill();
            let _ = state.child.wait();
            report_runtime(&model_id, "stopped", None, None, None);
        }
    }
}

pub fn llama_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install with `brew install llama.cpp`, or set NOLA_LLAMA_SERVER."
    } else if cfg!(windows) {
        "Install with `winget install llama.cpp`, or set NOLA_LLAMA_SERVER."
    } else {
        "Install llama.cpp and place llama-server in PATH, or set NOLA_LLAMA_SERVER."
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn process_observation_errors_are_not_treated_as_alive() {
        assert!(process_is_running(Ok(None)).unwrap());
        assert!(process_is_running(Err(std::io::Error::other("inspection unavailable"))).is_err());
    }

    #[cfg(unix)]
    struct LiveChildFixture(RuntimeState);

    #[cfg(unix)]
    impl Drop for LiveChildFixture {
        fn drop(&mut self) {
            let _ = self.0.child.kill();
            let _ = self.0.child.wait();
        }
    }

    #[cfg(unix)]
    fn live_child_fixture() -> LiveChildFixture {
        let child = Command::new("/bin/sh").arg("-c").arg("read pending")
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        LiveChildFixture(RuntimeState {
            child,
            launch: LocalModelLaunchSnapshot {
                model_id: "exact-local-profile".into(), binary_path: PathBuf::from("/fixture/engine"),
                artifact_path: PathBuf::from("/fixture/model.gguf"), parameters: LocalModelParameters::default(),
                server_subcommand: false,
            },
            instance_id: Uuid::new_v4().to_string(), port: 1, api_key: "isolated-fixture-key".into(),
        })
    }

    #[test]
    #[cfg(unix)]
    fn local_runtime_reuse_requires_identical_launch_facts_and_a_live_child() {
        let mut fixture = live_child_fixture();
        let original = fixture.0.launch.clone();
        assert!(fixture.0.can_reuse(&original).unwrap());
        let mut variants = Vec::new();
        macro_rules! changed_parameter {
            ($field:ident, $value:expr) => {{
                let mut changed = original.clone();
                changed.parameters.$field = $value;
                variants.push(changed);
            }};
        }
        changed_parameter!(context_tokens, 131072);
        changed_parameter!(temperature, 0.0);
        changed_parameter!(top_p, 0.5);
        changed_parameter!(top_k, 20);
        changed_parameter!(min_p, 0.0);
        changed_parameter!(batch_size, 256);
        changed_parameter!(threads, 2);
        changed_parameter!(gpu_layers, 0);
        changed_parameter!(flash_attention, false);
        let mut binary = original.clone();
        binary.binary_path = PathBuf::from("/fixture/other-engine");
        variants.push(binary);
        let mut invocation = original.clone();
        invocation.server_subcommand = true;
        variants.push(invocation);
        let mut artifact = original.clone();
        artifact.artifact_path = PathBuf::from("/fixture/other.gguf");
        variants.push(artifact);
        let mut model = original.clone();
        model.model_id = "different-profile".into();
        variants.push(model);
        for changed in variants {
            assert!(!fixture.0.can_reuse(&changed).unwrap(), "reused changed launch: {changed:?}");
        }
        assert!(fixture.0.can_reuse(&original).unwrap(), "comparison must not mutate the running process");
        drop(fixture.0.child.stdin.take());
        fixture.0.child.wait().unwrap();
        assert!(!fixture.0.can_reuse(&original).unwrap());
        assert!(fixture.0.live_snapshot(&original.model_id).unwrap().is_none());
    }

    #[test]
    #[cfg(unix)]
    fn local_live_snapshot_has_exact_identity_frozen_context_and_unique_instance() {
        let mut first = live_child_fixture();
        let mut second = live_child_fixture();
        let id = first.0.launch.model_id.clone();
        assert!(first.0.live_snapshot("Exact-Local-Profile").unwrap().is_none());
        let first_snapshot = first.0.live_snapshot(&id).unwrap().unwrap();
        let second_snapshot = second.0.live_snapshot(&id).unwrap().unwrap();
        assert_ne!(first_snapshot.endpoint.instance_id, second_snapshot.endpoint.instance_id);
        assert_eq!(first_snapshot.endpoint.context_window_tokens, first_snapshot.launch.parameters.context_tokens);
        assert_eq!(first_snapshot.launch, first.0.launch);
        assert_eq!(first_snapshot.endpoint.model_id, id);
        assert_eq!(first_snapshot.endpoint.instance_id, first.0.live_snapshot(&id).unwrap().unwrap().endpoint.instance_id);
        let mut new_preferences = first_snapshot.launch.parameters.clone();
        new_preferences.context_tokens *= 2;
        assert_eq!(first.0.live_snapshot(&id).unwrap().unwrap().endpoint.context_window_tokens, LOCAL_CONTEXT_TOKENS);
        assert_ne!(new_preferences.context_tokens, first_snapshot.endpoint.context_window_tokens);
    }

    #[test]
    fn catalog_has_stable_models_and_64k_context() {
        let ids = LOCAL_MODELS
            .iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["qwen3.5-0.8b-q4", "qwen3.5-4b-q4km"]);
        assert_eq!(LOCAL_CONTEXT_TOKENS, 65_536);
        assert!(LOCAL_MODELS.iter().all(|model| {
            model.download_url.starts_with("https://huggingface.co/")
                && model.download_bytes > 0
                && model.estimated_runtime_bytes > model.download_bytes
                && model.native_context_tokens >= LOCAL_CONTEXT_TOKENS
        }));
    }

    #[test]
    fn catalog_json_exposes_runtime_contract() {
        let value = catalog_json();
        assert_eq!(value["backend"], "llama.cpp");
        assert_eq!(value["minimumContextWindowTokens"], 65_536);
        assert!(value["diskUsageBytes"].as_u64().is_some());
        assert_eq!(value["models"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn profile_resolution_is_exact() {
        assert_eq!(
            profile("Qwen3.5 0.8B").map(|model| model.id),
            Some("qwen3.5-0.8b-q4")
        );
        assert_eq!(
            profile("qwen3.5-4b-q4km").map(|model| model.tier),
            Some("recommended")
        );
        assert!(profile("qwen3.5-random").is_none());
    }

    #[test]
    fn local_parameters_enforce_64k_and_bound_sampling_values() {
        let model = &LOCAL_MODELS[1];
        let too_small = LocalModelParameters {
            context_tokens: 32_768,
            ..LocalModelParameters::default()
        };
        assert!(too_small.validated(model).is_err());

        let bounded = LocalModelParameters {
            context_tokens: 999_999,
            temperature: 9.0,
            top_p: 4.0,
            top_k: 999,
            min_p: -2.0,
            batch_size: 99_999,
            threads: 999,
            gpu_layers: -5,
            flash_attention: true,
        }
        .validated(model)
        .unwrap();
        assert_eq!(bounded.context_tokens, model.native_context_tokens);
        assert_eq!(bounded.temperature, 2.0);
        assert_eq!(bounded.top_p, 1.0);
        assert_eq!(bounded.top_k, 200);
        assert_eq!(bounded.min_p, 0.0);
        assert_eq!(bounded.batch_size, 4096);
        assert_eq!(bounded.threads, 256);
        assert_eq!(bounded.gpu_layers, 0);
    }

    #[test]
    fn catalog_uses_one_consistent_recommended_quantization() {
        assert!(LOCAL_MODELS.iter().all(|model| {
            model.repository.starts_with("bartowski/Qwen_Qwen3.5-")
                && model.filename.ends_with("Q4_K_M.gguf")
        }));
    }

    #[test]
    fn dynamic_catalog_contract_runs_in_an_isolated_process() {
        if std::env::var_os("NOLA_DYNAMIC_CATALOG_CHILD").is_none() {
            let root = std::env::temp_dir().join(format!("nola-dynamic-model-{}", Uuid::new_v4()));
            let status = Command::new(std::env::current_exe().unwrap())
                .arg("local_models::tests::dynamic_catalog_contract_runs_in_an_isolated_process")
                .arg("--exact")
                .arg("--nocapture")
                .env("NOLA_DYNAMIC_CATALOG_CHILD", "1")
                .env("NOLA_HOME", &root)
                .env("NOLA_DB_PATH", root.join("state.db"))
                .env("NOLA_MAC_APP_SUPPORT", root.join("mac"))
                .env_remove("NOLA_APPLICATION_SERVICE_EXECUTABLE")
                .status()
                .unwrap();
            let _ = fs::remove_dir_all(root);
            assert!(status.success());
            return;
        }

        let root = models_root().unwrap();
        let snapshot = root.join("publisher-model/snapshots/revision-task");
        fs::create_dir_all(&snapshot).unwrap();
        let artifact = snapshot.join("model.gguf");
        fs::write(&artifact, b"verified dynamic fixture").unwrap();
        let catalog = json!({
            "schemaVersion": 2,
            "models": [{
                "id": "publisher-model",
                "name": "Publisher Model",
                "repository": "publisher/model",
                "revision": "revision",
                "runtime": "llama.cpp",
                "format": "gguf",
                "quantization": "Q4_K_M",
                "contextWindowTokens": 131072,
                "artifactPath": artifact,
                "artifactBytes": 24,
                "state": "configured",
                "enabled": true
            }]
        });
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("catalog.json"),
            serde_json::to_vec_pretty(&catalog).unwrap(),
        )
        .unwrap();

        assert!(contains_model("publisher-model"));
        assert!(model_ids().iter().any(|id| id == "publisher-model"));
        save_parameters("publisher-model", LocalModelParameters::default()).unwrap();
        let store = NolaStore::open_default().unwrap();
        assert!(
            !store
                .model_profile("publisher-model")
                .unwrap()
                .unwrap()
                .selected
        );
        select_model("publisher-model").unwrap();
        assert_eq!(selected_model_id().as_deref(), Some("publisher-model"));
        let mut canonical = store.model_profile("publisher-model").unwrap().unwrap();
        canonical.source_json = r#"{"repository":"canonical/model","revision":"newer"}"#.into();
        store.upsert_model_profile(canonical.clone()).unwrap();
        let saved = save_parameters(
            "publisher-model",
            LocalModelParameters {
                context_tokens: 131_072,
                ..LocalModelParameters::default()
            },
        )
        .unwrap();
        assert_eq!(saved.context_tokens, 131_072);
        let configured = store.model_profile("publisher-model").unwrap().unwrap();
        assert!(configured.selected);
        assert_eq!(configured.source_json, canonical.source_json);
        assert_eq!(configured.created_at_ms, canonical.created_at_ms);
        let catalog = catalog_json();
        let model = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "publisher-model")
            .unwrap();
        assert_eq!(model["installed"], true);
        assert_eq!(model["selected"], true, "catalog selection must follow the canonical model profile");
        assert_eq!(model["runtimeSupported"], true);

        // Aliases must resolve before reading the canonical version.
        save_parameters("Publisher Model", LocalModelParameters::default()).unwrap();
        let snapshot = catalog_json();
        let snapshot = snapshot["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "publisher-model")
            .unwrap();
        let revision = snapshot["configurationRevision"].as_u64().unwrap();
        let before = store.model_profile("publisher-model").unwrap().unwrap();
        assert_eq!(revision, before.revision as u64);
        assert_eq!(
            snapshot["parameters"],
            serde_json::from_str::<Value>(&before.parameters_json).unwrap()
        );
        save_parameters_at_revision(
            "publisher-model",
            LocalModelParameters {
                temperature: 0.2,
                ..LocalModelParameters::default()
            },
            revision,
        )
        .unwrap();
        let winner = store.model_profile("publisher-model").unwrap().unwrap();
        let error = save_parameters_at_revision(
            "Publisher Model",
            LocalModelParameters {
                temperature: 0.9,
                ..LocalModelParameters::default()
            },
            revision,
        )
        .unwrap_err();
        assert!(error.starts_with("revision_conflict:"), "{error}");
        assert_eq!(
            store.model_profile("publisher-model").unwrap().unwrap(),
            winner
        );
        let fresh = catalog_json();
        let fresh = fresh["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "publisher-model")
            .unwrap();
        assert_eq!(fresh["configurationRevision"], winner.revision);
        assert_eq!(fresh["parameters"]["temperature"], 0.2);

        remove_model("publisher-model").unwrap();
        assert!(!contains_model("publisher-model"));
        assert!(NolaStore::open_default()
            .unwrap()
            .model_profile("publisher-model")
            .unwrap()
            .is_none());
        let built_in_id = "qwen3.5-4b-q4km";
        save_parameters(built_in_id, LocalModelParameters::default()).unwrap();
        store
            .delete_model_profile(built_in_id, unix_epoch_millis())
            .unwrap();
        assert!(save_parameters(built_in_id, LocalModelParameters::default()).is_err());
        assert!(store.model_profile(built_in_id).unwrap().is_none());
    }
}
