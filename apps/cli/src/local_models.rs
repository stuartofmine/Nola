// SPDX-License-Identifier: MIT
// Shared desktop model catalog and inference engine lifecycle for CLI and Web.

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::task_runtime::OwnedShellChild;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    child: OwnedShellChild,
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
    pub engine: String,
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
    pub fn validated(self, model: &LocalModelProfile) -> Result<Self, String> {
        self.for_context(model.native_context_tokens, LOCAL_CONTEXT_TOKENS)
    }

    fn for_context(mut self, native: usize, minimum: usize) -> Result<Self, String> {
        if self.context_tokens < minimum {
            return Err(format!(
                "contextTokens must be at least {minimum}"
            ));
        }
        self.context_tokens = self.context_tokens.min(native);
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
    #[serde(default = "unreported_context_window", deserialize_with = "read_installed_context")]
    context_window_tokens: usize,
    recommended_context_tokens: Option<usize>,
    recommended_device: Option<String>,
    artifact_path: String,
    snapshot_path: Option<PathBuf>,
    artifact_bytes: u64,
    estimated_peak_memory_bytes: Option<u64>,
    recommended_memory_bytes: Option<u64>,
    state: String,
    enabled: bool,
}

impl InstalledModelRecord {
    fn initial_parameters(&self) -> LocalModelParameters {
        let context = self.recommended_context_tokens.filter(|&n| n > 0).unwrap_or(LOCAL_CONTEXT_TOKENS);
        LocalModelParameters {
            context_tokens: context.min(self.context_window_tokens.max(1)),
            gpu_layers: if self.runtime == "llama.cpp" && self.recommended_device.as_deref() == Some("cpu") {
                0
            } else {
                LocalModelParameters::default().gpu_layers
            },
            ..LocalModelParameters::default()
        }
    }
}

// The model-install Skill writes null when the repository does not report a window.
fn unreported_context_window() -> usize { 4096 }

fn read_installed_context<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    Ok(Option::<usize>::deserialize(deserializer)?.filter(|&n| n > 0).unwrap_or_else(unreported_context_window))
}

static DOWNLOADS: OnceLock<Mutex<HashMap<String, DownloadState>>> = OnceLock::new();
static DOWNLOAD_CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
static RUNTIME: OnceLock<Mutex<Option<RuntimeState>>> = OnceLock::new();

fn downloads() -> &'static Mutex<HashMap<String, DownloadState>> {
    DOWNLOADS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn download_cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    DOWNLOAD_CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
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
    for model in load_installed_catalog().unwrap_or_default().models {
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
    if let Some(root) = std::env::var_os("NOLA_MODELS_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("NOLA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".nola")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".nola")))
        .ok_or_else(|| "Cannot resolve the Nola home directory".to_string())?;
    Ok(home.join("models").join("llama.cpp"))
}

fn load_installed_catalog() -> Result<InstalledModelCatalog, String> {
    let catalog = crate::model_acquisition::read_catalog(&models_root()?)?;
    serde_json::from_value(catalog).map_err(|error| format!("Invalid local model catalog: {error}"))
}

fn installed_record(model_id: &str) -> Option<InstalledModelRecord> {
    let value = model_id.trim();
    let records: Vec<_> = load_installed_catalog().unwrap_or_default().models.into_iter()
        .filter(|model| model.enabled && model.state == "configured").collect();
    if let Some(model) = records.iter().find(|model| model.id.eq_ignore_ascii_case(value)) {
        return Some(model.clone());
    }
    let mut matches = records.into_iter().filter(|model|
        model.name.eq_ignore_ascii_case(value) || model.repository.eq_ignore_ascii_case(value));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

pub fn model_runtime(id: &str) -> Option<String> {
    if profile(id).is_some() { Some("llama.cpp".into()) } else { installed_record(id).map(|m| m.runtime) }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFiles {
    #[serde(default = "first_snapshot_version")]
    schema_version: u64,
    artifacts: Vec<SnapshotFile>,
    total_bytes: u64,
}

fn first_snapshot_version() -> u64 { 1 }

#[derive(Deserialize)]
struct SnapshotFile {
    path: PathBuf,
    bytes: u64,
}

fn checked_model_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = path.canonicalize().map_err(|e| format!("Model file unavailable: {e}"))?;
    if path == root || !path.starts_with(root) { return Err("Model file outside its snapshot".into()); }
    Ok(path)
}

fn validated_installed_path(model: &InstalledModelRecord) -> Result<PathBuf, String> {
    let is_vllm = model.runtime == "vllm" && model.format == "safetensors";
    if !(is_vllm || (model.runtime == "llama.cpp" && model.format == "gguf")) {
        return Err(format!("Unsupported engine/format: {} / {}", model.runtime, model.format));
    }
    validated_engine_files(model, &models_root()?.canonicalize().map_err(|e| e.to_string())?, is_vllm)
}

fn validated_engine_files(model: &InstalledModelRecord, root: &Path, is_vllm: bool) -> Result<PathBuf, String> {
    let artifact = checked_model_path(root, Path::new(&model.artifact_path))?;
    if let Some(snapshot) = &model.snapshot_path {
        let directory = checked_model_path(root, snapshot)?;
        let manifest = checked_model_path(&directory, &directory.join("snapshot.json"))?;
        let files: SnapshotFiles = serde_json::from_reader(fs::File::open(manifest).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid model snapshot: {e}"))?;
        if files.schema_version != 1 {
            return Err(format!("Unsupported local model snapshot version: {}", files.schema_version));
        }
        if files.artifacts.is_empty() || files.total_bytes != model.artifact_bytes {
            return Err("Incomplete model snapshot".into());
        }
        let mut total = 0u64;
        let mut includes_artifact = false;
        let mut includes_config = false;
        for file in files.artifacts {
            let path = checked_model_path(&directory, &directory.join(&file.path))?;
            let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() != file.bytes { return Err(format!("Model file changed: {}", file.path.display())); }
            total = total.checked_add(file.bytes).ok_or("Invalid model size")?;
            includes_artifact |= path == artifact;
            includes_config |= file.path == Path::new("config.json");
        }
        if total != files.total_bytes || !includes_artifact || (is_vllm && !includes_config) {
            return Err("Incomplete model snapshot".into());
        }
        if is_vllm { crate::model_acquisition::validate_vllm_directory(&directory)?; }
        return Ok(if is_vllm { directory } else { artifact });
    }
    if is_vllm || !artifact.is_file() || fs::metadata(&artifact).map_err(|e| e.to_string())?.len() != model.artifact_bytes {
        return Err("Incomplete installed model".into());
    }
    Ok(artifact)
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
    let built_in = profile(model_id);
    let installed = if built_in.is_none() { installed_record(model_id) } else { None };
    let id = built_in.map(|model| model.id)
        .or_else(|| installed.as_ref().map(|model| model.id.as_str()))
        .ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    let stored = NolaStore::open_default()
        .and_then(|store| store.model_profile(id))
        .ok()
        .flatten()
        .and_then(|profile| serde_json::from_str::<LocalModelParameters>(&profile.parameters_json).ok())
        .or_else(|| load_preferences().parameters.get(id).cloned());
    if let Some(model) = installed {
        return stored.unwrap_or_else(|| model.initial_parameters())
            .for_context(model.context_window_tokens.max(1), 1);
    }
    stored.unwrap_or_default().validated(built_in.expect("model id resolved above"))
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
        .unwrap_or(LOCAL_CONTEXT_TOKENS).max(1);
    let value = value.for_context(native_context, if built_in.is_some() { LOCAL_CONTEXT_TOKENS } else { 1 })?;
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
    let (display_name, model_name, runtime, source_json) = if let Some(profile) = profile(model_id) {
        (
            profile.name.to_string(),
            profile.id.to_string(),
            "llama.cpp".to_owned(),
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
            profile.runtime,
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
        runtime,
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
        if path.is_absolute() && executable_file(&path) {
            return Some(path);
        }
    }
    let candidates: &[&str] = if cfg!(windows) {
        &["llama-server.exe", "llama.exe"]
    } else {
        &["llama-server", "llama"]
    };
    if let Some(path_value) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path_value).filter(|dir| dir.is_absolute()) {
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

pub(crate) fn vllm_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NOLA_VLLM_BIN").filter(|val| !val.is_empty()).map(PathBuf::from) {
        return (path.is_absolute() && executable_file(&path)).then_some(path);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let name = if cfg!(windows) { "vllm.exe" } else { "vllm" };
    std::env::split_paths(&path).filter(|d| d.is_absolute()).map(|dir| dir.join(name))
        .chain(["/opt/homebrew/bin/vllm", "/usr/local/bin/vllm"].into_iter()
            .filter(|_| cfg!(target_os = "macos")).map(PathBuf::from))
        .find(|path| executable_file(path))
}

const VLLM_INSTALL_HINT: &str = "Install vLLM for your hardware and set NOLA_VLLM_BIN to its executable.";

pub(crate) fn executable_file(path: &Path) -> bool {
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
    #[cfg(windows)]
    unsafe {
        let mut status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX =
            std::mem::zeroed();
        status.dwLength = std::mem::size_of::<
            windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX,
        >() as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut status) != 0
            && status.ullTotalPhys > 0
        {
            return Some(status.ullTotalPhys);
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
    let installed_catalog = load_installed_catalog();
    let configuration_error = canonical.as_ref().err().or(installed_catalog.as_ref().err()).cloned();
    let canonical: HashMap<String, ModelProfile> = canonical
        .unwrap_or_default()
        .into_iter()
        .map(|profile| (profile.id.clone(), profile))
        .collect();
    let selected_id = canonical.values().find(|profile| {
        profile.provider == "local" && profile.selected && profile.enabled && profile.deleted_at_ms.is_none()
    }).map(|profile| profile.id.as_str()).or(preferences.selected_model_id.as_deref());
    let vllm_available = vllm_path().is_some();
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
                "runtime": "llama.cpp",
                "format": "gguf",
                "binaryAvailable": binary.is_some(),
                "installHint": llama_install_hint(),
                "downloadBytes": model.download_bytes,
                "estimatedRuntimeBytes": model.estimated_runtime_bytes,
                "recommendedMemoryBytes": model.recommended_memory_bytes,
                "contextWindowTokens": LOCAL_CONTEXT_TOKENS,
                "nativeContextTokens": model.native_context_tokens,
                "minimumContextWindowTokens": LOCAL_CONTEXT_TOKENS,
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
    for model in installed_catalog.unwrap_or_default().models {
        if LOCAL_MODELS
            .iter()
            .any(|built_in| built_in.id.eq_ignore_ascii_case(&model.id))
        {
            continue;
        }
        let path_result = validated_installed_path(&model);
        let runtime_supported = matches!((model.runtime.as_str(), model.format.as_str()), ("llama.cpp", "gguf") | ("vllm", "safetensors"));
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
            "runtime": model.runtime,
            "format": model.format,
            "binaryAvailable": if model.runtime == "vllm" { vllm_available } else { runtime_supported && binary.is_some() },
            "installHint": if model.runtime == "vllm" { VLLM_INSTALL_HINT } else { llama_install_hint() },
            "downloadBytes": model.artifact_bytes,
            "estimatedRuntimeBytes": model.estimated_peak_memory_bytes,
            "recommendedMemoryBytes": model.recommended_memory_bytes.or(model.estimated_peak_memory_bytes),
            "contextWindowTokens": model.context_window_tokens,
            "nativeContextTokens": model.context_window_tokens,
            "minimumContextWindowTokens": 1,
            "installed": installed,
            "running": running_id.as_deref() == Some(model.id.as_str()),
            "selected": selected_id == Some(model.id.as_str()),
            "deviceCompatible": runtime_supported,
            "runtimeSupported": runtime_supported,
            "runtimeIssue": path_result.err(),
            "download": DownloadState { status: if installed { "installed".to_string() } else { "invalid".to_string() }, downloaded_bytes: if installed { model.artifact_bytes } else { 0 }, total_bytes: model.artifact_bytes, error: None },
            "parameters": canonical.get(&model.id).and_then(|profile| serde_json::from_str::<Value>(&profile.parameters_json).ok()).or_else(|| preferences.parameters.get(&model.id).map(|parameters| json!(parameters))).unwrap_or_else(|| json!(model.initial_parameters())),
            "configurationRevision": if configuration_error.is_none() { canonical.get(&model.id).map_or(Some(0), |profile| u64::try_from(profile.revision).ok()) } else { None },
        }));
    }
    json!({
        "backend": "llama.cpp",
        "runtimes": [
            {"id":"llama.cpp","binaryAvailable":binary.is_some(),"installHint":llama_install_hint()},
            {"id":"vllm","binaryAvailable":vllm_available,"installHint":VLLM_INSTALL_HINT}
        ],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadOutcome {
    Completed,
    Cancelled,
}

pub fn start_download(model_id: &str) -> Result<(), String> {
    let model = *profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    if is_installed(&model) {
        return Ok(());
    }
    let cancel_flag = Arc::new(AtomicBool::new(false));
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
    if let Ok(mut cancels) = download_cancels().lock() {
        cancels.insert(model.id.to_string(), Arc::clone(&cancel_flag));
    }
    let worker_cancel = Arc::clone(&cancel_flag);
    if let Err(error) = thread::Builder::new()
        .name(format!("nola-model-download-{}", model.id))
        .spawn(move || {
            let result = download_model_inner_with_cancel(&model, Some(&worker_cancel), |downloaded, total| {
                if worker_cancel.load(Ordering::Acquire) {
                    return;
                }
                if let Ok(mut states) = downloads().lock() {
                    if worker_cancel.load(Ordering::Acquire) {
                        return;
                    }
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
            if let Ok(mut cancels) = download_cancels().lock() {
                if cancels
                    .get(model.id)
                    .is_some_and(|existing| Arc::ptr_eq(existing, &worker_cancel))
                {
                    cancels.remove(model.id);
                }
            }
            if worker_cancel.load(Ordering::Acquire) || matches!(result, Ok(DownloadOutcome::Cancelled)) {
                if let Ok(mut states) = downloads().lock() {
                    states.remove(model.id);
                }
                return;
            }
            if let Ok(mut states) = downloads().lock() {
                let (status, downloaded_bytes, error) = match result {
                    Ok(DownloadOutcome::Completed) => ("installed", model.download_bytes, None),
                    Ok(DownloadOutcome::Cancelled) => return,
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
    {
        if let Ok(mut cancels) = download_cancels().lock() {
            if cancels
                .get(model.id)
                .is_some_and(|existing| Arc::ptr_eq(existing, &cancel_flag))
            {
                cancels.remove(model.id);
            }
        }
        if let Ok(mut states) = downloads().lock() {
            states.remove(model.id);
        }
        return Err(format!("Failed to start model download: {error}"));
    }
    Ok(())
}

pub fn download_model<F>(model_id: &str, progress: F) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let model = profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    download_model_inner_with_cancel(model, None, progress).map(|_| ())
}

fn download_model_inner_with_cancel<F>(
    model: &LocalModelProfile,
    cancel: Option<&AtomicBool>,
    mut progress: F,
) -> Result<DownloadOutcome, String>
where
    F: FnMut(u64, u64),
{
    let destination = model_path(model)?;
    if is_installed(model) {
        progress(model.download_bytes, model.download_bytes);
        return Ok(DownloadOutcome::Completed);
    }
    download_model_from_url_with_cancel(
        model.download_url,
        &destination,
        model.download_bytes,
        cancel,
        progress,
    )
}

fn download_model_from_url_with_cancel<F>(
    download_url: &str,
    destination: &Path,
    expected_bytes: u64,
    cancel: Option<&AtomicBool>,
    mut progress: F,
) -> Result<DownloadOutcome, String>
where
    F: FnMut(u64, u64),
{
    let is_cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::Acquire));
    if is_cancelled() {
        return Ok(DownloadOutcome::Cancelled);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Invalid local model destination".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create model directory: {error}"))?;
    let partial = destination.with_extension("gguf.part");
    let mut existing = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 60 * 8))
        .build()
        .map_err(|error| format!("Failed to initialize model downloader: {error}"))?;
    let mut request = client.get(download_url);
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let mut response = request
        .send()
        .map_err(|error| format!("Model download failed: {error}"))?;
    if existing > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = fs::remove_file(&partial);
        existing = 0;
        if is_cancelled() {
            return Ok(DownloadOutcome::Cancelled);
        }
        response = client
            .get(download_url)
            .send()
            .map_err(|error| format!("Model download failed: {error}"))?;
    }
    if is_cancelled() {
        let _ = fs::remove_file(&partial);
        return Ok(DownloadOutcome::Cancelled);
    }
    if !response.status().is_success() {
        return Err(format!(
            "Model download failed with HTTP {}",
            response.status()
        ));
    }
    let resumed = existing > 0 && response.status().as_u16() == 206;
    let mut downloaded = if resumed { existing } else { 0 };
    let response_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| expected_bytes.saturating_sub(downloaded));
    let total = if resumed {
        downloaded.saturating_add(response_length)
    } else {
        response_length.max(expected_bytes)
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
        if is_cancelled() {
            drop(file);
            let _ = fs::remove_file(&partial);
            return Ok(DownloadOutcome::Cancelled);
        }
        let count = match response.read(&mut buffer) {
            Ok(count) => count,
            Err(_) if is_cancelled() => {
                drop(file);
                let _ = fs::remove_file(&partial);
                return Ok(DownloadOutcome::Cancelled);
            }
            Err(error) => return Err(format!("Failed while downloading model: {error}")),
        };
        if count == 0 {
            break;
        }
        if is_cancelled() {
            drop(file);
            let _ = fs::remove_file(&partial);
            return Ok(DownloadOutcome::Cancelled);
        }
        if let Err(error) = file.write_all(&buffer[..count]) {
            if is_cancelled() {
                drop(file);
                let _ = fs::remove_file(&partial);
                return Ok(DownloadOutcome::Cancelled);
            }
            return Err(format!("Failed to save model: {error}"));
        }
        downloaded = downloaded.saturating_add(count as u64);
        progress(downloaded, total);
    }
    if is_cancelled() {
        drop(file);
        let _ = fs::remove_file(&partial);
        return Ok(DownloadOutcome::Cancelled);
    }
    file.flush()
        .map_err(|error| format!("Failed to finish model download: {error}"))?;
    drop(file);
    if is_cancelled() {
        let _ = fs::remove_file(&partial);
        return Ok(DownloadOutcome::Cancelled);
    }
    if downloaded < expected_bytes {
        return Err(format!(
            "Model download is incomplete: {downloaded} of {expected_bytes} bytes"
        ));
    }
    if is_cancelled() {
        let _ = fs::remove_file(&partial);
        return Ok(DownloadOutcome::Cancelled);
    }
    fs::rename(&partial, destination)
        .map_err(|error| format!("Failed to install downloaded model: {error}"))?;
    Ok(DownloadOutcome::Completed)
}

fn partial_download_size(model: &LocalModelProfile) -> u64 {
    model_path(model)
        .ok()
        .map(|path| path.with_extension("gguf.part"))
        .and_then(|path| fs::metadata(path).ok())
        .map(|value| value.len())
        .unwrap_or(0)
}

fn engine_command(launch: &LocalModelLaunchSnapshot, port: u16, api_key: &str) -> Result<Command, String> {
    let p = &launch.parameters;
    let mut command = Command::new(&launch.binary_path);
    match launch.engine.as_str() {
        "vllm" => {
            command.arg("serve").arg(&launch.artifact_path)
                .arg("--served-model-name").arg(&launch.model_id)
                .arg("--max-model-len").arg(p.context_tokens.to_string())
                .arg("--generation-config").arg("vllm")
                .arg("--override-generation-config").arg(json!({
                    "temperature":p.temperature, "top_p":p.top_p, "top_k":p.top_k, "min_p":p.min_p
                }).to_string())
                .env("HF_HUB_OFFLINE", "1");
        }
        "llama.cpp" => {
            if launch.server_subcommand { command.arg("serve"); }
            command.arg("--model").arg(&launch.artifact_path)
                .arg("--alias").arg(&launch.model_id)
                .arg("--ctx-size").arg(p.context_tokens.to_string())
                .arg("--parallel").arg("1")
                .arg("--batch-size").arg(p.batch_size.to_string())
                .arg("--n-gpu-layers").arg(p.gpu_layers.to_string())
                .arg("--temp").arg(p.temperature.to_string())
                .arg("--top-p").arg(p.top_p.to_string())
                .arg("--top-k").arg(p.top_k.to_string())
                .arg("--min-p").arg(p.min_p.to_string())
                .arg("--jinja");
            if p.threads > 0 { command.arg("--threads").arg(p.threads.to_string()); }
            if p.flash_attention { command.args(["--flash-attn", "on"]); }
        }
        engine => return Err(format!("Unsupported local engine: {engine}")),
    }
    command.args(["--host", "127.0.0.1", "--port"]).arg(port.to_string()).arg("--api-key").arg(api_key)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    Ok(command)
}

fn wait_for_local_server_ready(child: &mut OwnedShellChild, port: u16, model: &str, engine: &str, key: &str,
    timeout: Duration) -> Result<(), String> {
    let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2)).build().map_err(|e| e.to_string())?;
    let url = format!("http://127.0.0.1:{port}/v1/models");
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("Cannot inspect {engine}: {e}"))? {
            return Err(format!("{engine} exited before loading {model}: {status}"));
        }
        if let Ok(response) = client.get(&url).bearer_auth(key).send().and_then(|r| r.error_for_status()) {
            if let Ok(catalog) = serde_json::from_reader::<_, Value>(response.take(1024 * 1024)) {
                if catalog["data"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"].as_str() == Some(model))) {
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline { return Err(format!("{engine} did not load {model} within {} seconds", timeout.as_secs())); }
        thread::sleep(Duration::from_millis(250));
    }
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
    let engine = installed.as_ref().map_or("llama.cpp", |m| m.runtime.as_str());
    let binary = if engine == "vllm" {
        vllm_path().ok_or_else(|| VLLM_INSTALL_HINT.to_string())?
    } else {
        llama_server_path().ok_or_else(|| llama_install_hint().to_string())?
    };
    let parameters = parameters(&resolved_id)?;
    let launch = LocalModelLaunchSnapshot {
        model_id: resolved_id.clone(),
        engine: engine.to_owned(),
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
        state.child.stop_and_confirm(Duration::from_secs(5))?;
        report_runtime(&state.launch.model_id, "stopped", None, None, None);
        *guard = None;
    }

    let port = available_loopback_port()?;
    let api_key = Uuid::new_v4().simple().to_string();
    let mut command = engine_command(&launch, port, &api_key)?;
    report_runtime(&resolved_id, "starting", None, None, None);
    let mut child = OwnedShellChild::spawn_owned(&mut command).map_err(|error| {
        let message = format!(
            "Failed to launch {engine} at {}: {error}",
            binary.display()
        );
        report_runtime(&resolved_id, "failed", None, None, Some(message.clone()));
        message
    })?;
    if let Err(message) = wait_for_local_server_ready(&mut child, port, &resolved_id, engine, &api_key, Duration::from_secs(180)) {
        let _ = child.stop_and_confirm(Duration::from_secs(5));
        report_runtime(&resolved_id, "failed", None, None, Some(message.clone()));
        return Err(message);
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

/// Remove only the named snapshot, under the same installation and catalog locks as acquisition.
pub fn remove_installed_snapshot(id: &str) -> Result<(), String> {
    let model = installed_record(id).filter(|m| m.id == id).ok_or("Unknown installed model")?;
    if Path::new(id).components().count() != 1 || !matches!(Path::new(id).components().next(), Some(std::path::Component::Normal(_))) {
        return Err("Invalid installed model id".into());
    }
    let root = models_root()?.canonicalize().map_err(|e| e.to_string())?;
    let owner = root.join(id);
    if owner.canonicalize().map_err(|e| e.to_string())? != owner {
        return Err("Model installation moved".into());
    }
    let _lock = crate::model_acquisition::model_lock(&owner)?;
    let snapshot = match &model.snapshot_path {
        Some(path) => checked_model_path(&owner, path)?,
        None => checked_model_path(&owner, Path::new(&model.artifact_path))?
            .parent().ok_or("Invalid model directory")?.to_owned(),
    };
    if snapshot.parent() != Some(owner.join("snapshots").as_path()) {
        return Err("Cannot remove a directory outside this model's snapshots".into());
    }
    let quarantine = tempfile::tempdir_in(&root).map_err(|e| e.to_string())?;
    let retired = quarantine.path().join("snapshot");
    let result = crate::model_acquisition::update_catalog(&root, |items| {
        let index = items.iter().position(|row| row["id"].as_str() == Some(id)
            && row["artifactPath"].as_str() == Some(&model.artifact_path))
            .ok_or("Model installation changed; reload before removing it")?;
        fs::rename(&snapshot, &retired).map_err(|e| e.to_string())?;
        items.remove(index);
        Ok(())
    });
    if result.is_err() && retired.exists() {
        if let Err(e) = fs::rename(&retired, &snapshot) {
            return Err(format!("Could not restore snapshot: {e}. Files preserved at {}", quarantine.keep().display()));
        }
    }
    result
}

pub fn remove_model(model_id: &str) -> Result<(), String> {
    if profile(model_id).is_none() {
        let model = installed_record(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
        stop_model(Some(&model.id))?;
        remove_installed_snapshot(&model.id)?;
        NolaStore::open_default()?.delete_model_profile(&model.id, unix_epoch_millis())?;
        return Ok(());
    }
    let model = profile(model_id).ok_or_else(|| format!("Unknown local model: {model_id}"))?;
    stop_model(Some(model.id))?;
    if let Some(cancel_flag) = download_cancels()
        .lock()
        .ok()
        .and_then(|mut cancels| cancels.remove(model.id))
    {
        cancel_flag.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_millis(500);
        while Arc::strong_count(&cancel_flag) > 1 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }
    if let Ok(mut states) = downloads().lock() {
        states.remove(model.id);
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

pub fn stop_model(expected_id: Option<&str>) -> Result<(), String> {
    let mut guard = runtime().lock().map_err(|_| "Local engine state is unavailable")?;
    if let Some(state) = guard.as_mut() {
        if expected_id.is_some_and(|id| id != state.launch.model_id) { return Ok(()); }
        state.child.stop_and_confirm(Duration::from_secs(5))?;
        report_runtime(&state.launch.model_id, "stopped", None, None, None);
        *guard = None;
    }
    Ok(())
}

pub fn shutdown() {
    if let Err(error) = stop_model(None) { eprintln!("Could not stop local model: {error}"); }
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
    fn each_engine_serves_the_catalog_id_from_its_own_model_format() {
        let mut launch = LocalModelLaunchSnapshot {
            model_id: "local-chat".into(), engine: "llama.cpp".into(), binary_path: "server".into(),
            artifact_path: "a model.gguf".into(), server_subcommand: false,
            parameters: LocalModelParameters { context_tokens: 131072, ..Default::default() },
        };
        let args = |s: &LocalModelLaunchSnapshot| engine_command(s, 9000, "fixture-token").unwrap()
            .get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let llama = args(&launch);
        assert!(llama.windows(2).any(|a| a == ["--model", "a model.gguf"]));
        assert!(llama.windows(2).any(|a| a == ["--alias", "local-chat"]));
        assert!(llama.windows(2).any(|a| a == ["--ctx-size", "131072"]));
        assert!(llama.windows(2).any(|a| a == ["--parallel", "1"]));
        launch.engine = "vllm".into();
        launch.artifact_path = "my snapshot".into();
        let vllm = args(&launch);
        assert_eq!(&vllm[..4], ["serve", "my snapshot", "--served-model-name", "local-chat"]);
        assert!(vllm.windows(2).any(|a| a == ["--max-model-len", "131072"]));
        assert!(!vllm.iter().any(|a| a == "--n-gpu-layers"));
        for engine in [llama, vllm] {
            assert!(engine.windows(2).any(|a| a == ["--host", "127.0.0.1"]));
            assert!(engine.windows(2).any(|a| a == ["--api-key", "fixture-token"]));
        }
    }

    #[test]
    fn a_skill_snapshot_with_unknown_context_does_not_hide_other_models() {
        let catalog: InstalledModelCatalog = serde_json::from_value(json!({"models":[
            {"id":"skill","contextWindowTokens":null},{"id":"hub","contextWindowTokens":2048}
        ]})).unwrap();
        assert_eq!(catalog.models[0].context_window_tokens, 4096);
        assert_eq!(catalog.models[1].context_window_tokens, 2048);
    }

    #[test]
    fn multi_file_model_validates_every_shard_and_uses_engine_specific_entry_point() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let directory = root.join("snapshot");
        fs::create_dir(&directory).unwrap();
        let mut artifacts = Vec::new();
        for name in ["config.json", "a", "b"] {
            fs::write(directory.join(name), b"{}").unwrap();
            artifacts.push(json!({"path":name,"bytes":2}));
        }
        fs::write(directory.join("snapshot.json"), json!({"totalBytes":6,"artifacts":artifacts}).to_string()).unwrap();
        let first = directory.join("a");
        let record = InstalledModelRecord { artifact_bytes: 6, snapshot_path: Some(directory.clone()),
            artifact_path: first.display().to_string(), ..Default::default() };
        assert_eq!(validated_engine_files(&record, &root, true).unwrap(), directory);
        assert_eq!(validated_engine_files(&record, &root, false).unwrap(), first);
        let manifest = directory.join("snapshot.json");
        fs::write(&manifest, json!({"schemaVersion":99,"totalBytes":6,"artifacts":artifacts}).to_string()).unwrap();
        assert!(validated_engine_files(&record, &root, true).unwrap_err().contains("version"));
        fs::write(&manifest, json!({"schemaVersion":1,"totalBytes":6,"artifacts":artifacts}).to_string()).unwrap();
        fs::write(directory.join("b"), b"").unwrap();
        assert!(validated_engine_files(&record, &root, true).is_err());
    }

    #[test]
    fn process_observation_errors_are_not_treated_as_alive() {
        assert!(process_is_running(Ok(None)).unwrap());
        assert!(process_is_running(Err(std::io::Error::other("inspection unavailable"))).is_err());
    }

    #[cfg(unix)]
    struct LiveChildFixture(RuntimeState);

    #[cfg(unix)]
    fn live_child_fixture() -> LiveChildFixture {
        let child = OwnedShellChild::spawn_owned(Command::new("/bin/sh").arg("-c").arg("read pending")
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())).unwrap();
        LiveChildFixture(RuntimeState {
            child,
            launch: LocalModelLaunchSnapshot {
                model_id: "exact-local-profile".into(), binary_path: PathBuf::from("/fixture/engine"),
                artifact_path: PathBuf::from("/fixture/model.gguf"), parameters: LocalModelParameters::default(),
                server_subcommand: false, engine: "llama.cpp".into(),
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
        fixture.0.child.stop_and_confirm(Duration::from_secs(5)).unwrap();
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
            "schemaVersion": 1,
            "models": [{
                "id": "publisher-model",
                "name": "Publisher Model",
                "repository": "publisher/model",
                "revision": "revision",
                "runtime": "llama.cpp",
                "format": "gguf",
                "quantization": "Q4_K_M",
                "contextWindowTokens": 131072,
                "recommendedContextTokens": 3072,
                "recommendedDevice": "cpu",
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
        assert_eq!(parameters("publisher/model").unwrap().context_tokens, 3072);
        let catalog = catalog_json();
        let initial = catalog["models"].as_array().unwrap().iter()
            .find(|m| m["id"] == "publisher-model").unwrap();
        assert_eq!(initial["parameters"]["contextTokens"], 3072);
        assert_eq!(initial["parameters"]["gpuLayers"], 0);
        assert_eq!(initial["nativeContextTokens"], 131072);
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
        assert_eq!(parameters("Publisher Model").unwrap().context_tokens, 131_072);
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

    #[test]
    fn download_recovers_from_http_416_by_removing_stale_part_and_retrying() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("model.gguf");
        let partial = destination.with_extension("gguf.part");
        // Write a stale partial file larger than the remote payload to trigger HTTP 416.
        fs::write(&partial, b"stale-oversized-partial-file-contents").unwrap();

        let payload = b"fresh-gguf-payload";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            // Request 1: has Range header -> respond 416
            {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap();
                requests.push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                write!(
                    stream,
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
            // Request 2: retry without Range -> respond 200 with full payload
            {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap();
                requests.push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                )
                .unwrap();
                stream.write_all(payload).unwrap();
            }
            requests
        });

        let url = format!("http://{addr}/model.gguf");
        let outcome = download_model_from_url_with_cancel(
            &url,
            &destination,
            payload.len() as u64,
            None,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome, DownloadOutcome::Completed);
        assert_eq!(fs::read(&destination).unwrap(), payload);
        assert!(!partial.exists());

        let requests = server.join().unwrap();
        assert!(requests[0].contains("range: bytes="), "first request should send Range: {}", requests[0]);
        assert!(!requests[1].contains("range:"), "retry request must not send Range: {}", requests[1]);
    }

    #[test]
    fn download_cancellation_exits_cleanly_and_cleans_up_partial_file() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("model.gguf");
        let partial = destination.with_extension("gguf.part");
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_progress = Arc::clone(&cancel);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 32\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(b"0123456789abcdef");
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(50));
            let _ = stream.write_all(b"0123456789abcdef");
        });

        let url = format!("http://{addr}/model.gguf");
        let outcome = download_model_from_url_with_cancel(
            &url,
            &destination,
            32,
            Some(&cancel),
            move |downloaded, _| {
                if downloaded > 0 {
                    cancel_for_progress.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        let _ = server.join();
        assert_eq!(outcome, DownloadOutcome::Cancelled);
        assert!(!destination.exists(), "cancelled download must not install destination");
        assert!(!partial.exists(), "cancelled download must remove partial file");
    }
}
