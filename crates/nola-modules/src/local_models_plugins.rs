// SPDX-License-Identifier: Apache-2.0
//! Local Models hardware probe, engine runtime process manager, recommendations,
//! and Harness Plugins / Desktop Plugin Pages catalog.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use nola_store::Store;
use nola_tools::{path_within_workspace, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, CapabilityDomain, LocalModelRecord, McpServerConfig, McpTransport,
    PluginRecord, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use crate::ModulesSuite;

const MAX_PROBE_BODY_BYTES: u64 = 256 * 1024;
const MAX_PLUGIN_HTML_BYTES: usize = 512 * 1024;
const MAX_PLUGIN_STAGED_FILES: usize = 64;
const MAX_PLUGIN_STAGED_FILE_BYTES: u64 = 4 * 1024 * 1024;
const KV_MODEL_DOWNLOAD_STATUS: &str = "local_model_download_status";

pub const KIND_LOCAL_MODEL: &str = "local_model";
pub const KIND_PLUGIN: &str = "plugin";

/// Probe real host hardware specs (OS, arch, CPU, cores, RAM, free disk, GPU).
/// Matches both native diagnostics and `LocalModelSearch.svelte` (`LocalHardware`).
pub fn probe_hardware() -> Value {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let cpu_model = detect_cpu_model();
    let (memory_bytes, available_memory_bytes) = detect_memory_bytes();
    let free_disk_bytes = detect_free_disk_bytes();
    let (gpu_name, gpu_memory_bytes, unified_memory) = detect_gpu(memory_bytes);

    json!({
        "os": os,
        "arch": arch,
        "cpu": cpu_model,
        "cores": cpu_cores,
        "cpu_cores": cpu_cores,
        "memoryBytes": memory_bytes,
        "availableMemoryBytes": available_memory_bytes,
        "freeDiskBytes": free_disk_bytes,
        "gpu": gpu_name,
        "gpuMemoryBytes": gpu_memory_bytes,
        "unifiedMemory": unified_memory
    })
}

fn detect_cpu_model() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = fs::read_to_string("/proc/cpuinfo") {
            for line in raw.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    if key == "model name" || key == "Hardware" || key == "Processor" {
                        let val = v.trim();
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    Some(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
}

fn detect_memory_bytes() -> (Option<u64>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = fs::read_to_string("/proc/meminfo") {
            let mut total_kib: Option<u64> = None;
            let mut avail_kib: Option<u64> = None;
            for line in raw.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    total_kib = rest
                        .split_whitespace()
                        .next()
                        .and_then(|n| n.parse::<u64>().ok());
                } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    avail_kib = rest
                        .split_whitespace()
                        .next()
                        .and_then(|n| n.parse::<u64>().ok());
                }
            }
            return (
                total_kib.map(|k| k.saturating_mul(1024)),
                avail_kib.map(|k| k.saturating_mul(1024)),
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        let total = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok());
        return (total, total);
    }
    #[allow(unreachable_code)]
    (None, None)
}

fn detect_free_disk_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let out = Command::new("df")
            .args(["-P", "-k", "."])
            .env("LC_ALL", "C")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let data_line = stdout.lines().skip(1).find(|l| !l.trim().is_empty())?;
        let cols: Vec<&str> = data_line.split_whitespace().collect();
        // Standard POSIX `df -k`: Filesystem 1024-blocks Used Available Capacity Mounted
        if cols.len() >= 4 {
            if let Ok(avail_kib) = cols[3].parse::<u64>() {
                return Some(avail_kib.saturating_mul(1024));
            }
        }
    }
    None
}

fn detect_gpu(total_memory_bytes: Option<u64>) -> (Option<String>, Option<u64>, bool) {
    if let Ok(out) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = stdout.lines().find(|l| !l.trim().is_empty()) {
                let mut parts = first.split(',');
                let name = parts.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let vram_mib = parts
                    .next()
                    .and_then(|s| s.trim().parse::<u64>().ok());
                if name.is_some() {
                    return (
                        name,
                        vram_mib.map(|m| m.saturating_mul(1024 * 1024)),
                        false,
                    );
                }
            }
        }
    }
    if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
        return (Some("Apple Silicon GPU".into()), total_memory_bytes, true);
    }
    (None, None, false)
}

/// Locate an executable binary by explicit path or searching `PATH`.
pub fn find_binary_on_path(binary: &str) -> Option<PathBuf> {
    let trimmed = binary.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = Path::new(trimmed);
    if candidate.components().count() > 1 {
        return is_executable_file(candidate).then(|| candidate.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(trimmed);
        if is_executable_file(&direct) {
            return Some(direct);
        }
        #[cfg(windows)]
        {
            let with_exe = dir.join(format!("{trimmed}.exe"));
            if is_executable_file(&with_exe) {
                return Some(with_exe);
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Truthfully probe whether a local inference engine binary is installed and ready.
/// NEVER fakes `ready: true` when the binary is absent.
pub fn probe_engine_status(engine: &str) -> Value {
    let norm = engine.trim().to_ascii_lowercase();
    let backend_env = std::env::var("NOLA_LLAMA_BACKEND")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (canonical_engine, env_override, bin_candidates, guide_url, default_backend): (
        &str,
        Option<String>,
        &[&str],
        Option<&str>,
        &str,
    ) = match norm.as_str() {
        "llama.cpp" | "llama-server" | "llama" => (
            "llama.cpp",
            std::env::var("NOLA_LLAMA_SERVER").ok(),
            &["llama-server"],
            Some("https://github.com/ggml-org/llama.cpp"),
            if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
                "metal"
            } else {
                "cpu"
            },
        ),
        "ollama" => (
            "ollama",
            std::env::var("NOLA_OLLAMA_BIN").ok(),
            &["ollama"],
            None,
            "ollama",
        ),
        "vllm" => (
            "vllm",
            std::env::var("NOLA_VLLM_BIN").ok(),
            &["vllm"],
            Some("https://docs.vllm.ai/en/latest/getting_started/installation.html"),
            "cuda",
        ),
        "mlx" | "mlx_lm" | "mlx_lm.server" => (
            "mlx",
            std::env::var("NOLA_MLX_SERVER").ok(),
            &["mlx_lm.server"],
            None,
            "metal",
        ),
        "lmstudio" | "lm-studio" | "lm_studio" | "lms" => (
            "lmstudio",
            std::env::var("NOLA_LMSTUDIO_BIN").ok(),
            &["lms"],
            Some("https://lmstudio.ai/download"),
            "lmstudio",
        ),
        _ => (
            engine.trim(),
            None,
            &[],
            None,
            "unknown",
        ),
    };

    let mut binary_path: Option<PathBuf> = None;
    if let Some(raw) = env_override.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        binary_path = find_binary_on_path(raw);
    }
    if binary_path.is_none() {
        for cand in bin_candidates {
            if let Some(found) = find_binary_on_path(cand) {
                binary_path = Some(found);
                break;
            }
        }
    }

    let mut lmstudio_endpoint: Option<String> = None;
    let mut lmstudio_reachable = false;
    if canonical_engine == "lmstudio" {
        let ep = std::env::var("NOLA_LMSTUDIO_ENDPOINT")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:1234/v1".to_string());
        let probe_url = format!("{ep}/models");
        if let Ok(parsed) = Url::parse(&probe_url) {
            let is_loopback = match parsed.host() {
                Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                Some(Host::Ipv4(v4)) => v4.is_loopback(),
                Some(Host::Ipv6(v6)) => v6.is_loopback(),
                None => false,
            };
            if is_loopback {
                if let Ok(resp) = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_millis(400))
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .and_then(|c| c.get(parsed).send())
                {
                    if resp.status().is_success() {
                        lmstudio_reachable = true;
                    }
                }
            }
        }
        lmstudio_endpoint = Some(ep);
    }

    let ready = binary_path.is_some() || lmstudio_reachable;
    let backend = if canonical_engine == "llama.cpp" {
        backend_env.unwrap_or_else(|| default_backend.to_string())
    } else {
        default_backend.to_string()
    };
    let can_install = matches!(
        canonical_engine,
        "llama.cpp" | "ollama" | "vllm" | "mlx" | "lmstudio"
    );
    let message = match &binary_path {
        Some(p) => format!("{canonical_engine} ready at {}", p.display()),
        None if lmstudio_reachable => format!(
            "lmstudio server reachable at {}",
            lmstudio_endpoint.as_deref().unwrap_or("http://127.0.0.1:1234/v1")
        ),
        None if bin_candidates.is_empty() => format!("unsupported local model engine '{engine}'"),
        None => format!(
            "{canonical_engine} binary ({}) not found on PATH",
            bin_candidates.join(" / ")
        ),
    };

    let reason = if ready {
        "ready"
    } else if bin_candidates.is_empty() {
        "unsupported"
    } else if env_override
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        "invalid_override"
    } else if canonical_engine == "lmstudio" && !lmstudio_reachable {
        "not_running"
    } else {
        "binary_not_found"
    };

    json!({
        "engine": canonical_engine,
        "ready": ready,
        "binaryPath": binary_path.map(|p| p.display().to_string()),
        "endpoint": lmstudio_endpoint,
        "serverReachable": lmstudio_reachable,
        "backend": backend,
        "canInstall": can_install,
        "guideUrl": guide_url,
        "reason": reason,
        "message": message
    })
}

struct RunningLocalModel {
    id: String,
    engine: String,
    model: String,
    port: u16,
    endpoint: String,
    binary_path: String,
    started_at_ms: i64,
    child: Child,
}

/// Manages local inference server child processes (`llama-server`, `ollama`, `vllm`, `mlx_lm.server`).
pub struct LocalModelProcessManager {
    processes: Mutex<BTreeMap<String, RunningLocalModel>>,
}

impl Default for LocalModelProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalModelProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(BTreeMap::new()),
        }
    }

    /// Global singleton process manager shared by CLI/Web/Daemon hosts.
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<LocalModelProcessManager> = OnceLock::new();
        GLOBAL.get_or_init(LocalModelProcessManager::new)
    }

    pub fn start_local_model(
        &self,
        id: &str,
        engine: &str,
        model_path: &str,
        port: u16,
        extra_args: &[String],
    ) -> Result<Value, String> {
        let status = probe_engine_status(engine);
        let ready = status.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
        let Some(binary_path) = status
            .get("binaryPath")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
        else {
            return Err(status
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("local model engine binary not found")
                .to_string());
        };
        if !ready {
            return Err(format!("local model engine '{engine}' is not ready"));
        }
        let model_trimmed = model_path.trim();
        if model_trimmed.is_empty() {
            return Err("model_path must not be empty".into());
        }

        let canonical_engine = status
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or(engine);
        let effective_port = if port == 0 { 8081 } else { port };
        let entry_id = if id.trim().is_empty() {
            new_id()
        } else {
            id.trim().to_string()
        };

        // Reap exited entries, kill any existing process with the same `id`, and reject port conflicts
        // BEFORE spawning the new subprocess!
        let mut guard = self
            .processes
            .lock()
            .map_err(|_| "process manager lock poisoned".to_string())?;
        let mut exited_ids = Vec::new();
        for (existing_id, entry) in guard.iter_mut() {
            if matches!(entry.child.try_wait(), Ok(Some(_))) {
                exited_ids.push(existing_id.clone());
            }
        }
        for dead_id in exited_ids {
            guard.remove(&dead_id);
        }
        if let Some(mut old) = guard.remove(&entry_id) {
            let _ = old.child.kill();
            let _ = old.child.wait();
        }
        if let Some(conflict) = guard.values().find(|e| e.port == effective_port) {
            return Err(format!(
                "port {effective_port} is already held by running local model '{}'",
                conflict.id
            ));
        }

        let mut cmd = Command::new(&binary_path);
        match canonical_engine {
            "llama.cpp" => {
                cmd.args([
                    "--model",
                    model_trimmed,
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &effective_port.to_string(),
                ]);
            }
            "vllm" => {
                cmd.args([
                    "serve",
                    model_trimmed,
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &effective_port.to_string(),
                ]);
            }
            "mlx" => {
                cmd.args([
                    "--model",
                    model_trimmed,
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &effective_port.to_string(),
                ]);
            }
            "ollama" => {
                cmd.env("OLLAMA_HOST", format!("127.0.0.1:{effective_port}"));
                cmd.arg("serve");
            }
            "lmstudio" => {
                cmd.args(["server", "start", "--port", &effective_port.to_string()]);
            }
            other => {
                return Err(format!("cannot spawn unsupported local engine '{other}'"));
            }
        }
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn '{binary_path}': {e}"))?;
        let pid = child.id();
        let endpoint = format!("http://127.0.0.1:{effective_port}");
        let started_at_ms = now_ms();

        guard.insert(
            entry_id.clone(),
            RunningLocalModel {
                id: entry_id.clone(),
                engine: canonical_engine.to_string(),
                model: model_trimmed.to_string(),
                port: effective_port,
                endpoint: endpoint.clone(),
                binary_path: binary_path.clone(),
                started_at_ms,
                child,
            },
        );

        Ok(json!({
            "id": entry_id,
            "engine": canonical_engine,
            "model": model_trimmed,
            "port": effective_port,
            "endpoint": endpoint,
            "pid": pid,
            "binaryPath": binary_path,
            "startedAtMs": started_at_ms
        }))
    }

    pub fn stop_all(&self) {
        if let Ok(mut guard) = self.processes.lock() {
            for (_, mut proc_entry) in std::mem::take(&mut *guard) {
                let _ = proc_entry.child.kill();
                let _ = proc_entry.child.wait();
            }
        }
    }

    pub fn stop_local_model(&self, id: &str) -> Result<Value, String> {
        let mut guard = self
            .processes
            .lock()
            .map_err(|_| "process manager lock poisoned".to_string())?;
        if let Some(mut proc_entry) = guard.remove(id) {
            let _ = proc_entry.child.kill();
            let _ = proc_entry.child.wait();
            Ok(json!({
                "id": id,
                "stopped": true,
                "engine": proc_entry.engine,
                "port": proc_entry.port
            }))
        } else {
            Ok(json!({
                "id": id,
                "stopped": false
            }))
        }
    }

    pub fn list_running_models(&self) -> Vec<Value> {
        let Ok(mut guard) = self.processes.lock() else {
            return Vec::new();
        };
        let mut exited = Vec::new();
        let mut active = Vec::new();
        for (id, entry) in guard.iter_mut() {
            if let Ok(Some(_)) = entry.child.try_wait() {
                exited.push(id.clone());
                continue;
            }
            active.push(json!({
                "id": entry.id,
                "engine": entry.engine,
                "model": entry.model,
                "port": entry.port,
                "endpoint": entry.endpoint,
                "pid": entry.child.id(),
                "binaryPath": entry.binary_path,
                "startedAtMs": entry.started_at_ms
            }));
        }
        for id in exited {
            guard.remove(&id);
        }
        active
    }
}

/// Recommend open-weight local models fitted to the probed hardware memory budget.
/// Returns `{hardware, candidates, recommendationSource}` matching `LocalModelSearch.svelte`.
pub fn recommend_local_models(hardware: &Value) -> Value {
    let unified = hardware
        .get("unifiedMemory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let budget_bytes = if unified {
        hardware.get("memoryBytes").and_then(|v| v.as_u64())
    } else {
        hardware
            .get("gpuMemoryBytes")
            .and_then(|v| v.as_u64())
            .or_else(|| hardware.get("memoryBytes").and_then(|v| v.as_u64()))
    }
    .unwrap_or(8 * 1024 * 1024 * 1024);

    let device = if unified {
        "unified"
    } else if hardware.get("gpuMemoryBytes").and_then(|v| v.as_u64()).is_some() {
        "gpu"
    } else {
        "cpu"
    };

    let catalog = [
        (
            "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF",
            "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
            1_150_000_000u64,
            2_200_000_000u64,
            32_768u64,
        ),
        (
            "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
            "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
            4_700_000_000u64,
            7_200_000_000u64,
            32_768u64,
        ),
        (
            "unsloth/Qwen3-8B-GGUF",
            "Qwen3-8B-UD-Q4_K_XL.gguf",
            5_100_000_000u64,
            8_400_000_000u64,
            131_072u64,
        ),
        (
            "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
            "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
            9_000_000_000u64,
            13_500_000_000u64,
            65_536u64,
        ),
        (
            "Qwen/Qwen2.5-Coder-32B-Instruct-GGUF",
            "qwen2.5-coder-32b-instruct-q4_k_m.gguf",
            19_800_000_000u64,
            26_000_000_000u64,
            65_536u64,
        ),
    ];

    let max_fit = (budget_bytes as f64 * 0.85) as u64;
    let mut candidates: Vec<Value> = catalog
        .iter()
        .filter(|(_, _, _, est_mem, _)| *est_mem <= max_fit || *est_mem <= 2_500_000_000)
        .map(|(repo, variant, dl_bytes, est_mem, ctx)| {
            json!({
                "repository": repo,
                "revision": "main",
                "variant": variant,
                "quantized": true,
                "downloadBytes": dl_bytes,
                "estimatedMemoryBytes": est_mem,
                "contextTokens": ctx,
                "nativeContextTokens": ctx,
                "contextLimitReason": if *ctx >= 131_072 { "target_met" } else { "native_limit" },
                "plan": {
                    "engine": "llama.cpp",
                    "repository": repo,
                    "variant": variant,
                    "quantized": true,
                    "device": device,
                    "files": [{ "path": variant, "bytes": dl_bytes }]
                }
            })
        })
        .collect();

    // Sort largest well-fitting model first
    candidates.reverse();

    json!({
        "hardware": hardware,
        "candidates": candidates,
        "recommendationSource": "catalog"
    })
}

fn is_loopback_parsed_url(parsed: &Url) -> bool {
    match parsed.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(v4)) => v4.is_loopback(),
        Some(Host::Ipv6(v6)) => v6.is_loopback(),
        None => false,
    }
}

/// Real HuggingFace / HF-mirror model search (`/api/models?search=...&limit=10`) with
/// graceful offline fallback to hardware-curated recommendations.
pub fn search_hub_models(
    query: &str,
    engine: &str,
    mirror_base: Option<&str>,
) -> Result<Vec<Value>, String> {
    let q = query.trim();
    let eng_norm = engine.trim().to_ascii_lowercase();
    let base_raw = mirror_base
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            std::env::var("NOLA_HF_MIRROR")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "https://huggingface.co".to_string());

    let mut api_url = Url::parse(&format!("{}/api/models", base_raw.trim_end_matches('/')))
        .map_err(|e| format!("invalid hub mirror base URL '{base_raw}': {e}"))?;
    let effective_query = if q.is_empty() {
        match eng_norm.as_str() {
            "mlx" => "mlx",
            "llama.cpp" | "llama" | "lmstudio" => "gguf",
            _ => "qwen",
        }
    } else {
        q
    };
    api_url
        .query_pairs_mut()
        .append_pair("search", effective_query)
        .append_pair("limit", "10");

    let live_res: Option<Vec<Value>> = (|| {
        let is_loopback = is_loopback_parsed_url(&api_url);
        let client = if is_loopback {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Nola/2.0")
                .build()
                .ok()?
        } else {
            let (_parsed, host, addrs) =
                nola_tools::web::resolve_and_validate_public_url(api_url.as_str()).ok()?;
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(6))
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(&host, &addrs)
                .user_agent("Nola/2.0")
                .build()
                .ok()?
        };
        let resp = client.get(api_url.clone()).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let mut buf = Vec::new();
        resp.take(MAX_PROBE_BODY_BYTES).read_to_end(&mut buf).ok()?;
        let arr: Vec<Value> = serde_json::from_slice(&buf).ok()?;
        let mapped: Vec<Value> = arr
            .into_iter()
            .take(10)
            .filter_map(|item| {
                let repo_id = item
                    .get("id")
                    .or_else(|| item.get("modelId"))
                    .and_then(|v| v.as_str())?
                    .to_string();
                let tags: Vec<String> = item
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(ToString::to_string))
                            .take(12)
                            .collect()
                    })
                    .unwrap_or_default();
                Some(json!({
                    "id": repo_id,
                    "repository": repo_id,
                    "engine": if eng_norm.is_empty() { "llama.cpp" } else { eng_norm.as_str() },
                    "downloads": item.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0),
                    "likes": item.get("likes").and_then(|v| v.as_u64()).unwrap_or(0),
                    "tags": tags,
                    "url": format!("{}/{}", base_raw.trim_end_matches('/'), repo_id),
                    "source": "huggingface",
                    "offlineFallback": false
                }))
            })
            .collect();
        (!mapped.is_empty()).then_some(mapped)
    })();

    if let Some(items) = live_res {
        return Ok(items);
    }

    // Graceful offline fallback to curated hardware-fitted recommendations
    let hw = probe_hardware();
    let rec = recommend_local_models(&hw);
    let q_lower = q.to_ascii_lowercase();
    let all_candidates = rec
        .get("candidates")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut filtered: Vec<Value> = all_candidates
        .iter()
        .filter(|c| {
            if q_lower.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {}",
                c.get("repository").and_then(|v| v.as_str()).unwrap_or(""),
                c.get("variant").and_then(|v| v.as_str()).unwrap_or("")
            )
            .to_ascii_lowercase();
            q_lower.split_whitespace().any(|tok| hay.contains(tok))
        })
        .cloned()
        .collect();
    if filtered.is_empty() {
        filtered = all_candidates;
    }
    let fallback: Vec<Value> = filtered
        .into_iter()
        .map(|mut c| {
            if let Some(obj) = c.as_object_mut() {
                obj.insert("source".into(), json!("curated_fallback"));
                obj.insert("offlineFallback".into(), json!(true));
                if !eng_norm.is_empty() {
                    obj.insert("engine".into(), json!(eng_norm));
                }
            }
            c
        })
        .collect();
    Ok(fallback)
}

/// Download specification for `ModelDownloadManager::start_download`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadPlan {
    #[serde(default, alias = "download_id")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "default_download_engine")]
    pub engine: String,
    pub url: String,
    #[serde(alias = "dest_path")]
    pub dest_path: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub quantization: Option<String>,
    #[serde(default, alias = "expected_bytes")]
    pub expected_bytes: Option<u64>,
}

fn default_download_engine() -> String {
    "llama.cpp".into()
}

fn default_download_source() -> String {
    "huggingface".into()
}

fn derive_download_repo_and_source(parsed: &Url, fallback_name: &str) -> (String, String) {
    let host = parsed
        .host_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default();
    if host == "modelscope.cn" || host == "www.modelscope.cn" {
        let repo = if segments.len() >= 3 && segments[0].eq_ignore_ascii_case("models") {
            format!("{}/{}", segments[1], segments[2])
        } else {
            fallback_name.to_string()
        };
        return (repo, "modelscope".into());
    }
    let repo = if (host == "huggingface.co"
        || host == "www.huggingface.co"
        || host == "hf-mirror.com")
        && segments.len() >= 2
    {
        format!("{}/{}", segments[0], segments[1])
    } else {
        fallback_name.to_string()
    };
    (repo, "huggingface".into())
}

fn default_models_dir() -> PathBuf {
    for key in ["NOLA_MODELS_ROOT", "NOLA_MODELS_DIR"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    if let Ok(nola_home) = std::env::var("NOLA_HOME") {
        let trimmed = nola_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("models");
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".nola/models")
}

fn allowed_model_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for key in ["NOLA_MODELS_ROOT", "NOLA_MODELS_DIR"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                dirs.push(PathBuf::from(trimmed));
            }
        }
    }
    if let Ok(nola_home) = std::env::var("NOLA_HOME") {
        let trimmed = nola_home.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed).join("models"));
        }
    }
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed).join(".nola/models"));
        }
    }
    #[cfg(test)]
    {
        dirs.push(std::env::temp_dir());
    }
    if dirs.is_empty() {
        dirs.push(default_models_dir());
    }
    dirs
}

/// Safely join a relative model `filename` (or nested relative path with only normal segments)
/// inside `base_dir`, rejecting `..`, absolute paths, symlinks, symlink escapes, and sensitive paths.
pub fn safe_model_dest_path(base_dir: &Path, filename: &str) -> Result<PathBuf, String> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err("model destination filename must not be empty".into());
    }
    let rel = Path::new(trimmed);
    if rel.is_absolute()
        || rel.components().count() == 0
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!(
            "unsafe model destination filename '{trimmed}': must be a relative path with normal segments only"
        ));
    }
    let candidate = base_dir.join(rel);
    if crate::data_sources::is_sensitive_local_path(&candidate) {
        return Err(format!(
            "refused sensitive model destination path '{}'",
            candidate.display()
        ));
    }
    if !path_within_workspace(base_dir, &candidate) {
        return Err(format!(
            "model destination '{}' escapes base directory '{}'",
            candidate.display(),
            base_dir.display()
        ));
    }
    let mut cur = base_dir.to_path_buf();
    for comp in rel.components() {
        cur.push(comp);
        if let Ok(meta) = fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "model destination path '{}' traverses a symlink",
                    cur.display()
                ));
            }
        }
    }
    Ok(candidate)
}

fn validate_model_dest_path(dest_trimmed: &str) -> Result<PathBuf, String> {
    if dest_trimmed.is_empty() {
        return Err("dest_path must not be empty".into());
    }
    let raw = Path::new(dest_trimmed);
    if raw
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "dest_path '{dest_trimmed}' must not contain '..' traversal components"
        ));
    }
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        safe_model_dest_path(&default_models_dir(), dest_trimmed)?
    };
    if crate::data_sources::is_sensitive_local_path(&candidate) {
        return Err(format!(
            "refused sensitive model destination path '{}'",
            candidate.display()
        ));
    }
    let bases = allowed_model_base_dirs();
    let Some(matched_base) = bases
        .iter()
        .find(|base| path_within_workspace(base, &candidate))
    else {
        return Err(format!(
            "dest_path '{}' must reside inside the managed models directory",
            candidate.display()
        ));
    };
    if let Ok(rel) = candidate.strip_prefix(matched_base) {
        let mut cur = matched_base.clone();
        for comp in rel.components() {
            cur.push(comp);
            if let Ok(meta) = fs::symlink_metadata(&cur) {
                if meta.file_type().is_symlink() {
                    return Err(format!(
                        "dest_path '{}' traverses a symlink",
                        cur.display()
                    ));
                }
            }
        }
    }
    let part = PathBuf::from(format!("{}.part", candidate.display()));
    if let Ok(meta) = fs::symlink_metadata(&part) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "download staging file '{}' must not be a symlink",
                part.display()
            ));
        }
    }
    Ok(candidate)
}

fn validate_download_redirect_target(
    current: &Url,
    location: &str,
) -> Result<(Url, String, Vec<std::net::SocketAddr>), String> {
    let next = current
        .join(location.trim())
        .map_err(|e| format!("invalid redirect Location '{location}': {e}"))?;
    if next.scheme() != "https" {
        return Err(format!(
            "blocked redirect to non-HTTPS URL '{}' (model downloads require https:// on redirects)",
            next
        ));
    }
    nola_tools::web::resolve_and_validate_public_url(next.as_str())
}

/// Live & persisted download state exposed by `ModelDownloadManager`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadState {
    #[serde(default)]
    pub id: String,
    pub download_id: String,
    pub name: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default = "default_download_source")]
    pub source: String,
    pub engine: String,
    pub url: String,
    pub dest_path: String,
    pub part_path: String,
    pub status: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Resumable, SHA-256-verifying background local model downloader.
pub struct ModelDownloadManager {
    cancels: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    latest: Mutex<Option<ModelDownloadState>>,
}

impl Default for ModelDownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

fn compute_percent(downloaded: u64, total: Option<u64>) -> f64 {
    match total {
        Some(t) if t > 0 => ((downloaded as f64 / t as f64) * 100.0).min(100.0),
        _ => 0.0,
    }
}

fn persist_download_state(
    manager_latest: &Mutex<Option<ModelDownloadState>>,
    store: &Store,
    state: &ModelDownloadState,
) {
    if let Ok(mut guard) = manager_latest.lock() {
        *guard = Some(state.clone());
    }
    if let Ok(val) = serde_json::to_value(state) {
        let _ = store.set_kv(KV_MODEL_DOWNLOAD_STATUS, &val);
        let _ = store.set_kv(&format!("local_model_download:{}", state.download_id), &val);
    }
}

fn stream_file_sha256_hex(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("failed to open downloaded file: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("failed reading file for SHA-256: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

impl ModelDownloadManager {
    pub fn new() -> Self {
        Self {
            cancels: Mutex::new(BTreeMap::new()),
            latest: Mutex::new(None),
        }
    }

    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<ModelDownloadManager> = OnceLock::new();
        GLOBAL.get_or_init(ModelDownloadManager::new)
    }

    /// Start (or resume via `Range: bytes=<existing>-`) a background local model download.
    /// Verifies HTTPS (or loopback HTTP for local mirrors/tests), streams into `<dest>.part`,
    /// verifies optional SHA-256, atomically renames to `<dest>`, and registers `LocalModelRecord`.
    pub fn start_download(
        &'static self,
        store: &Store,
        plan: ModelDownloadPlan,
    ) -> Result<ModelDownloadState, String> {
        let parsed = Url::parse(plan.url.trim())
            .map_err(|e| format!("invalid model download URL: {e}"))?;
        let is_loopback = is_loopback_parsed_url(&parsed);
        if parsed.scheme() != "https" && !(is_loopback && parsed.scheme() == "http") {
            return Err("model download URL must use https:// (or loopback http://)".into());
        }
        let resolved_remote = if is_loopback {
            None
        } else {
            Some(nola_tools::web::resolve_and_validate_public_url(parsed.as_str())?)
        };

        let dest_trimmed = plan.dest_path.trim();
        let dest_path = validate_model_dest_path(dest_trimmed)?;
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create download directory: {e}"))?;
            }
        }
        // Re-validate after creating parent directories so no symlink appeared in parent chain.
        let dest_path = validate_model_dest_path(&dest_path.display().to_string())?;
        let part_path = PathBuf::from(format!("{}.part", dest_path.display()));

        let existing_bytes = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
        let download_id = plan
            .id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(new_id);
        let (repository, source) = derive_download_repo_and_source(&parsed, &plan.name);
        let started_at_ms = now_ms();
        let initial_state = ModelDownloadState {
            id: download_id.clone(),
            download_id: download_id.clone(),
            name: plan.name.clone(),
            repository,
            source,
            engine: plan.engine.clone(),
            url: plan.url.clone(),
            dest_path: dest_path.display().to_string(),
            part_path: part_path.display().to_string(),
            status: "downloading".into(),
            downloaded_bytes: existing_bytes,
            total_bytes: plan.expected_bytes,
            percent: compute_percent(existing_bytes, plan.expected_bytes),
            sha256: plan.sha256.clone(),
            error: None,
            model_id: None,
            started_at_ms,
            updated_at_ms: started_at_ms,
        };

        let cancel_flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut guard) = self.cancels.lock() {
            guard.insert(download_id.clone(), Arc::clone(&cancel_flag));
        }
        persist_download_state(&self.latest, store, &initial_state);

        let store_bg = store.clone();
        let mut state_bg = initial_state.clone();
        let plan_bg = plan;
        std::thread::Builder::new()
            .name(format!("nola-model-dl-{}", &download_id[..download_id.len().min(8)]))
            .spawn(move || {
                let outcome: Result<(), String> = (|| {
                    const MAX_REDIRECTS: usize = 5;
                    let mut current_url = parsed;
                    let mut current_is_loopback = is_loopback;
                    let mut current_remote = resolved_remote;
                    let mut resp_opt = None;

                    for hop in 0..=MAX_REDIRECTS {
                        let mut builder = reqwest::blocking::Client::builder()
                            .timeout(Duration::from_secs(300))
                            .redirect(reqwest::redirect::Policy::none())
                            .user_agent("Nola/2.0");
                        if current_is_loopback {
                            builder = builder.no_proxy();
                        } else if let Some((_, ref host, ref addrs)) = current_remote {
                            builder = builder.resolve_to_addrs(host, addrs);
                        }
                        let client = builder.build().map_err(|e| e.to_string())?;

                        let mut req = client.get(current_url.clone());
                        if existing_bytes > 0 {
                            req = req.header(
                                reqwest::header::RANGE,
                                format!("bytes={existing_bytes}-"),
                            );
                        }
                        let resp = req
                            .send()
                            .map_err(|e| format!("download request failed: {e}"))?;
                        let status_u16 = resp.status().as_u16();
                        if matches!(status_u16, 301 | 302 | 303 | 307 | 308) {
                            if hop == MAX_REDIRECTS {
                                return Err(format!(
                                    "download exceeded maximum of {MAX_REDIRECTS} redirects"
                                ));
                            }
                            let loc = resp
                                .headers()
                                .get(reqwest::header::LOCATION)
                                .and_then(|v| v.to_str().ok())
                                .ok_or_else(|| {
                                    format!("redirect HTTP {status_u16} missing Location header")
                                })?;
                            let (next_url, next_host, next_addrs) =
                                validate_download_redirect_target(&current_url, loc)?;
                            current_url = next_url.clone();
                            current_is_loopback = false;
                            current_remote = Some((next_url, next_host, next_addrs));
                            continue;
                        }
                        resp_opt = Some(resp);
                        break;
                    }

                    let mut resp = resp_opt
                        .ok_or_else(|| "download failed without final HTTP response".to_string())?;
                    let status_code = resp.status();

                    if status_code.as_u16() == 416 && existing_bytes > 0 {
                        // Existing `.part` is already at or beyond server size; proceed to verification.
                        state_bg.downloaded_bytes = existing_bytes;
                        state_bg.total_bytes = Some(existing_bytes);
                        state_bg.percent = 100.0;
                    } else if status_code.is_success() || status_code.as_u16() == 206 {
                        let is_partial = status_code.as_u16() == 206 && existing_bytes > 0;
                        let content_len = resp.content_length();
                        let total = if is_partial {
                            content_len
                                .map(|rem| existing_bytes.saturating_add(rem))
                                .or(plan_bg.expected_bytes)
                        } else {
                            content_len.or(plan_bg.expected_bytes)
                        };
                        let mut file = if is_partial {
                            state_bg.downloaded_bytes = existing_bytes;
                            OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&part_path)
                                .map_err(|e| format!("failed to open .part for resume: {e}"))?
                        } else {
                            state_bg.downloaded_bytes = 0;
                            OpenOptions::new()
                                .create(true)
                                .write(true)
                                .truncate(true)
                                .open(&part_path)
                                .map_err(|e| format!("failed to create .part file: {e}"))?
                        };
                        state_bg.total_bytes = total;
                        state_bg.percent = compute_percent(state_bg.downloaded_bytes, total);
                        state_bg.updated_at_ms = now_ms();
                        persist_download_state(&self.latest, &store_bg, &state_bg);

                        let mut buf = [0u8; 65_536];
                        let mut since_flush_bytes = 0u64;
                        let mut last_persist = Instant::now();
                        loop {
                            if cancel_flag.load(Ordering::Relaxed) {
                                state_bg.status = "cancelled".into();
                                state_bg.updated_at_ms = now_ms();
                                persist_download_state(&self.latest, &store_bg, &state_bg);
                                return Ok(());
                            }
                            let n = resp
                                .read(&mut buf)
                                .map_err(|e| format!("download stream error: {e}"))?;
                            if n == 0 {
                                break;
                            }
                            file.write_all(&buf[..n])
                                .map_err(|e| format!("disk write error: {e}"))?;
                            state_bg.downloaded_bytes =
                                state_bg.downloaded_bytes.saturating_add(n as u64);
                            since_flush_bytes = since_flush_bytes.saturating_add(n as u64);
                            if since_flush_bytes >= 262_144
                                || last_persist.elapsed() >= Duration::from_millis(200)
                            {
                                state_bg.percent =
                                    compute_percent(state_bg.downloaded_bytes, state_bg.total_bytes);
                                state_bg.updated_at_ms = now_ms();
                                persist_download_state(&self.latest, &store_bg, &state_bg);
                                since_flush_bytes = 0;
                                last_persist = Instant::now();
                            }
                        }
                        file.flush().map_err(|e| format!("disk flush error: {e}"))?;
                    } else {
                        return Err(format!("download server returned HTTP {status_code}"));
                    }

                    if cancel_flag.load(Ordering::Relaxed) {
                        state_bg.status = "cancelled".into();
                        state_bg.updated_at_ms = now_ms();
                        persist_download_state(&self.latest, &store_bg, &state_bg);
                        return Ok(());
                    }

                    // Verification phase
                    state_bg.status = "verifying".into();
                    state_bg.percent = compute_percent(
                        state_bg.downloaded_bytes,
                        state_bg.total_bytes.or(Some(state_bg.downloaded_bytes)),
                    );
                    state_bg.updated_at_ms = now_ms();
                    persist_download_state(&self.latest, &store_bg, &state_bg);

                    let computed_sha = stream_file_sha256_hex(&part_path)?;
                    if let Some(expected_sha) = plan_bg
                        .sha256
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        if !computed_sha.eq_ignore_ascii_case(expected_sha) {
                            let _ = fs::remove_file(&part_path);
                            return Err(format!(
                                "SHA-256 mismatch: expected {expected_sha}, got {computed_sha}"
                            ));
                        }
                    }
                    state_bg.sha256 = Some(computed_sha);

                    // Atomic rename & register local model in Store
                    fs::rename(&part_path, &dest_path)
                        .map_err(|e| format!("atomic rename failed: {e}"))?;
                    let final_size = fs::metadata(&dest_path)
                        .map(|m| m.len())
                        .unwrap_or(state_bg.downloaded_bytes);
                    let model_rec = LocalModelRecord {
                        id: new_id(),
                        name: plan_bg.name.clone(),
                        engine: plan_bg.engine.clone(),
                        path: Some(dest_path.display().to_string()),
                        size_bytes: final_size,
                        quantization: plan_bg.quantization.clone(),
                        installed: true,
                        updated_at_ms: now_ms(),
                    };
                    store_bg
                        .put_module_record(KIND_LOCAL_MODEL, &model_rec.id, &model_rec)
                        .map_err(|e| format!("failed to register local model: {e}"))?;

                    state_bg.status = "installed".into();
                    state_bg.downloaded_bytes = final_size;
                    state_bg.total_bytes = Some(final_size);
                    state_bg.percent = 100.0;
                    state_bg.model_id = Some(model_rec.id);
                    state_bg.updated_at_ms = now_ms();
                    persist_download_state(&self.latest, &store_bg, &state_bg);
                    Ok(())
                })();

                if let Err(err) = outcome {
                    state_bg.status = "failed".into();
                    state_bg.error = Some(err);
                    state_bg.updated_at_ms = now_ms();
                    persist_download_state(&self.latest, &store_bg, &state_bg);
                }
                if let Ok(mut guard) = self.cancels.lock() {
                    guard.remove(&state_bg.download_id);
                }
            })
            .map_err(|e| format!("failed to spawn download thread: {e}"))?;

        Ok(initial_state)
    }

    /// Cooperatively cancel an active download by ID (preserving `.part` for resume).
    pub fn cancel_download(&self, store: &Store, download_id: &str) -> Result<Value, String> {
        let mut found = false;
        if let Ok(guard) = self.cancels.lock() {
            if let Some(flag) = guard.get(download_id) {
                flag.store(true, Ordering::Relaxed);
                found = true;
            }
        }
        if let Ok(Some(mut st)) = store.get_kv(&format!("local_model_download:{download_id}")) {
            if let Some(obj) = st.as_object_mut() {
                if obj.get("status").and_then(|v| v.as_str()) == Some("downloading") {
                    obj.insert("status".into(), json!("cancelled"));
                    obj.insert("updatedAtMs".into(), json!(now_ms()));
                    let _ = store.set_kv(&format!("local_model_download:{download_id}"), &st);
                    let _ = store.set_kv(KV_MODEL_DOWNLOAD_STATUS, &st);
                }
            }
        }
        Ok(json!({
            "downloadId": download_id,
            "cancelRequested": found
        }))
    }

    /// Return the latest download status (preferring live in-memory state, falling back to KV).
    pub fn current_status(&self, store: &Store) -> Value {
        if let Ok(guard) = self.latest.lock() {
            if let Some(ref st) = *guard {
                if let Ok(v) = serde_json::to_value(st) {
                    return v;
                }
            }
        }
        let mut val = store
            .get_kv(KV_MODEL_DOWNLOAD_STATUS)
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({ "status": "idle" }));
        if let Some(obj) = val.as_object_mut() {
            let status = obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("idle");
            if status != "idle" {
                if status == "completed" {
                    obj.insert("status".into(), json!("installed"));
                }
                if !obj.contains_key("id") {
                    if let Some(dl_id) = obj.get("downloadId").cloned() {
                        obj.insert("id".into(), dl_id);
                    }
                }
                if obj
                    .get("repository")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    let name = obj
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let url_opt = obj
                        .get("url")
                        .and_then(|v| v.as_str())
                        .and_then(|u| Url::parse(u).ok());
                    let (repo, src) = match url_opt {
                        Some(u) => derive_download_repo_and_source(&u, &name),
                        None => (name, default_download_source()),
                    };
                    obj.insert("repository".into(), json!(repo));
                    obj.entry("source").or_insert_with(|| json!(src));
                } else {
                    obj.entry("source")
                        .or_insert_with(|| json!(default_download_source()));
                }
            }
        }
        val
    }
}

/// Check local inference engine presence and return real actionable installation steps
/// (or run the package installer when explicitly opted in via `opt_in_install` or
/// `NOLA_ALLOW_ENGINE_AUTO_INSTALL=1`). NEVER pretends an engine is ready when missing.
pub fn run_engine_setup(engine: &str) -> Value {
    let opt_in = std::env::var("NOLA_ALLOW_ENGINE_AUTO_INSTALL")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    run_engine_setup_with_opt_in(engine, opt_in)
}

pub fn run_engine_setup_with_opt_in(engine: &str, opt_in_install: bool) -> Value {
    let probed = probe_engine_status(engine);
    let canonical = probed
        .get("engine")
        .and_then(|v| v.as_str())
        .unwrap_or(engine)
        .to_string();
    let ready = probed.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
    if ready {
        return json!({
            "engine": canonical,
            "ready": true,
            "installed": true,
            "attemptedInstall": false,
            "status": probed,
            "steps": [],
            "message": probed.get("message").and_then(|v| v.as_str()).unwrap_or("engine ready")
        });
    }

    let is_macos = std::env::consts::OS == "macos";
    let (steps, auto_cmd): (Vec<&str>, Option<(&str, Vec<&str>)>) = match canonical.as_str() {
        "llama.cpp" => {
            if is_macos {
                (
                    vec![
                        "Install llama.cpp via Homebrew: brew install llama.cpp",
                        "Verify installation: llama-server --version",
                    ],
                    Some(("brew", vec!["install", "llama.cpp"])),
                )
            } else {
                (
                    vec![
                        "Build or install llama-server from https://github.com/ggml-org/llama.cpp",
                        "Ensure `llama-server` is on PATH or set NOLA_LLAMA_SERVER=/path/to/llama-server",
                    ],
                    None,
                )
            }
        }
        "ollama" => {
            if is_macos {
                (
                    vec![
                        "Install Ollama via Homebrew: brew install ollama",
                        "Start daemon: ollama serve",
                    ],
                    Some(("brew", vec!["install", "ollama"])),
                )
            } else {
                (
                    vec![
                        "Install Ollama: curl -fsSL https://ollama.com/install.sh | sh",
                        "Start daemon: ollama serve",
                    ],
                    None,
                )
            }
        }
        "vllm" => (
            vec![
                "Install vLLM in Python: pip3 install vllm",
                "Verify installation: vllm --help",
            ],
            Some(("pip3", vec!["install", "vllm"])),
        ),
        "mlx" => (
            vec![
                "Install MLX-LM on Apple Silicon: pip3 install mlx-lm",
                "Verify installation: mlx_lm.server --help",
            ],
            Some(("pip3", vec!["install", "mlx-lm"])),
        ),
        "lmstudio" => {
            if is_macos {
                (
                    vec![
                        "Install LM Studio: brew install --cask lm-studio (or download from https://lmstudio.ai/download)",
                        "Bootstrap CLI (`lms`) from LM Studio or start its local server on http://127.0.0.1:1234/v1",
                    ],
                    Some(("brew", vec!["install", "--cask", "lm-studio"])),
                )
            } else {
                (
                    vec![
                        "Download LM Studio from https://lmstudio.ai/download",
                        "Install `lms` CLI or start LM Studio Local Server on http://127.0.0.1:1234/v1",
                    ],
                    None,
                )
            }
        }
        _ => (
            vec!["Unsupported local engine; choose llama.cpp, ollama, vllm, mlx, or lmstudio."],
            None,
        ),
    };

    if opt_in_install {
        if let Some((bin, cmd_args)) = auto_cmd {
            if let Some(bin_path) = find_binary_on_path(bin) {
                match Command::new(&bin_path).args(&cmd_args).output() {
                    Ok(out) => {
                        let post = probe_engine_status(&canonical);
                        let post_ready =
                            post.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
                        return json!({
                            "engine": canonical,
                            "ready": post_ready,
                            "installed": post_ready,
                            "attemptedInstall": true,
                            "command": format!("{bin} {}", cmd_args.join(" ")),
                            "exitCode": out.status.code(),
                            "stdout": String::from_utf8_lossy(&out.stdout).chars().take(1_000).collect::<String>(),
                            "stderr": String::from_utf8_lossy(&out.stderr).chars().take(1_000).collect::<String>(),
                            "status": post,
                            "steps": steps,
                            "message": post.get("message").and_then(|v| v.as_str()).unwrap_or("install command finished")
                        });
                    }
                    Err(e) => {
                        return json!({
                            "engine": canonical,
                            "ready": false,
                            "installed": false,
                            "attemptedInstall": true,
                            "error": format!("failed to execute {bin}: {e}"),
                            "status": probed,
                            "steps": steps,
                            "message": format!("Automatic install via {bin} failed: {e}")
                        });
                    }
                }
            }
        }
    }

    json!({
        "engine": canonical,
        "ready": false,
        "installed": false,
        "attemptedInstall": false,
        "status": probed,
        "steps": steps,
        "message": probed.get("message").and_then(|v| v.as_str()).unwrap_or("engine not installed")
    })
}

/// Summary of reconciled enabled plugins' declared runtime contributions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRuntimeSummary {
    pub enabled_plugins: Vec<Value>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub skill_dirs: Vec<PathBuf>,
    pub pages: Vec<Value>,
    pub errors: Vec<String>,
}

fn sanitize_plugin_dir_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect()
}

fn stage_plugin_directory(src_dir: &Path, dest_dir: &Path) -> Result<(usize, String), String> {
    fs::create_dir_all(dest_dir).map_err(|e| format!("failed to create plugin dir: {e}"))?;
    let mut stack = vec![src_dir.to_path_buf()];
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    while let Some(cur) = stack.pop() {
        let entries = fs::read_dir(&cur).map_err(|e| format!("failed reading plugin source: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("failed reading entry metadata: {e}"))?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if files.len() >= MAX_PLUGIN_STAGED_FILES {
                    return Err(format!(
                        "plugin exceeds maximum of {MAX_PLUGIN_STAGED_FILES} files"
                    ));
                }
                if meta.len() > MAX_PLUGIN_STAGED_FILE_BYTES {
                    return Err(format!(
                        "plugin file '{}' exceeds 4 MiB limit",
                        path.display()
                    ));
                }
                let rel = path
                    .strip_prefix(src_dir)
                    .map_err(|_| "plugin file escaped source directory".to_string())?
                    .to_path_buf();
                if rel
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                {
                    return Err(format!("unsafe plugin relative path '{}'", rel.display()));
                }
                files.push((rel, path));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (rel, abs) in &files {
        let target = dest_dir.join(rel);
        if !path_within_workspace(dest_dir, &target) {
            return Err(format!("plugin path '{}' escapes destination", rel.display()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed creating dir: {e}"))?;
        }
        let bytes = fs::read(abs).map_err(|e| format!("failed reading plugin file: {e}"))?;
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update(&bytes);
        fs::write(&target, &bytes).map_err(|e| format!("failed writing staged plugin file: {e}"))?;
    }
    Ok((files.len(), hex::encode(hasher.finalize())))
}

fn validate_plugin_entry_rel(src_dir: &Path, entry_rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(entry_rel);
    if rel.is_absolute()
        || rel.components().count() == 0
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!(
            "unsafe plugin entry_path '{entry_rel}': must be a relative path with normal segments only"
        ));
    }
    let mut cur = src_dir.to_path_buf();
    for comp in rel.components() {
        cur.push(comp);
        if let Ok(meta) = fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "unsafe plugin entry_path '{entry_rel}': symlink component rejected"
                ));
            }
        }
    }
    if !path_within_workspace(src_dir, &cur) {
        return Err(format!(
            "unsafe plugin entry_path '{entry_rel}': escapes plugin directory"
        ));
    }
    Ok(rel.to_path_buf())
}

/// Install a plugin package either by staging a local directory/manifest into
/// `<workspace>/.nola/plugins/<name>/` (with SHA-256 verification) OR by invoking
/// `pi`/`dsh` CLI when present on `PATH`, then recording `PluginRecord` in `Store`.
pub fn install_plugin_package(
    store: &Store,
    workspace_root: &Path,
    runtime: &str,
    profile: &str,
    source: &str,
) -> Result<PluginRecord, String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("plugin source must not be empty".into());
    }
    let rt = if runtime.trim().is_empty() {
        "dsh"
    } else {
        runtime.trim()
    };
    let prof = if profile.trim().is_empty() {
        if rt == "pi" { "pi" } else { "web" }
    } else {
        profile.trim()
    };

    let src_path = Path::new(src_trimmed);
    if src_path.exists() {
        let (src_dir, manifest_path) = if src_path.is_file() {
            let parent = src_path
                .parent()
                .ok_or_else(|| "plugin manifest has no parent directory".to_string())?;
            (parent.to_path_buf(), src_path.to_path_buf())
        } else {
            (src_path.to_path_buf(), src_path.join("nola-plugin.json"))
        };

        let raw_manifest = fs::read_to_string(&manifest_path).map_err(|e| {
            format!(
                "local plugin missing readable nola-plugin.json at '{}': {e}",
                manifest_path.display()
            )
        })?;
        let manifest: Value = serde_json::from_str(&raw_manifest)
            .map_err(|e| format!("invalid nola-plugin.json: {e}"))?;

        let raw_id = manifest
            .get("id")
            .or_else(|| manifest.get("name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| src_dir.file_name().and_then(|s| s.to_str()))
            .unwrap_or("plugin");
        let safe_name = sanitize_plugin_dir_name(raw_id);
        if safe_name.is_empty() {
            return Err("could not derive valid plugin directory name".into());
        }

        // Verify optional entry file path safety & SHA-256 digest declared in manifest before staging
        let entry_rel = manifest
            .get("entry_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let validated_entry_rel = if !entry_rel.is_empty() {
            Some(validate_plugin_entry_rel(&src_dir, &entry_rel)?)
        } else {
            None
        };
        if let Some(expected_sha) = manifest
            .get("sha256")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let target_to_hash = if let Some(ref rel) = validated_entry_rel {
                src_dir.join(rel)
            } else {
                manifest_path.clone()
            };
            let bytes = fs::read(&target_to_hash)
                .map_err(|e| format!("failed to read plugin file for sha256 check: {e}"))?;
            let actual = sha256_hex(&bytes);
            if !actual.eq_ignore_ascii_case(expected_sha) {
                return Err(format!(
                    "plugin SHA-256 digest mismatch: expected {expected_sha}, got {actual}"
                ));
            }
        }

        let dest_dir = workspace_root.join(".nola/plugins").join(&safe_name);
        let (_file_count, bundle_sha) = stage_plugin_directory(&src_dir, &dest_dir)?;

        let name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&safe_name)
            .to_string();
        let version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0")
            .to_string();
        let mut capabilities: Vec<String> = manifest
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let rt_cap = format!("runtime:{rt}");
        if !capabilities.contains(&rt_cap) {
            capabilities.push(rt_cap);
        }
        let prof_cap = format!("profile:{prof}");
        if !capabilities.contains(&prof_cap) {
            capabilities.push(prof_cap);
        }
        capabilities.push(format!("sha256:{bundle_sha}"));

        let rec = PluginRecord {
            id: safe_name,
            name,
            version,
            entry_path: if entry_rel.is_empty() {
                dest_dir.join("nola-plugin.json").display().to_string()
            } else {
                dest_dir.join(&entry_rel).display().to_string()
            },
            enabled: manifest
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            capabilities,
        };
        store
            .put_module_record(KIND_PLUGIN, &rec.id, &rec)
            .map_err(|e| e.to_string())?;
        return Ok(rec);
    }

    // Reject CLI flag injection (e.g. `--...` / `-...`) before invoking `pi`/`dsh`
    if src_trimmed.starts_with('-') {
        return Err(format!(
            "invalid plugin source '{src_trimmed}': package source must not start with '-'"
        ));
    }

    // Remote / package-manager install via `pi` or `dsh` CLI if present on PATH
    let cli_bin = match rt.to_ascii_lowercase().as_str() {
        "pi" => "pi",
        "dsh" => "dsh",
        other => {
            return Err(format!(
                "plugin source '{src_trimmed}' not found locally and unsupported runtime '{other}'"
            ));
        }
    };
    let Some(bin_path) = find_binary_on_path(cli_bin) else {
        return Err(format!(
            "plugin source '{src_trimmed}' does not exist locally and '{cli_bin}' CLI is not installed on PATH"
        ));
    };
    let out = Command::new(&bin_path)
        .args(["install", src_trimmed])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| format!("failed to invoke '{cli_bin}': {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "'{cli_bin} install {src_trimmed}' failed (exit {:?}): {}",
            out.status.code(),
            stderr.trim()
        ));
    }

    let safe_id = sanitize_plugin_dir_name(src_trimmed);
    let id = if safe_id.is_empty() { new_id() } else { safe_id };
    let rec = PluginRecord {
        id: id.clone(),
        name: src_trimmed.to_string(),
        version: "0.1.0".into(),
        entry_path: String::new(),
        enabled: true,
        capabilities: vec![format!("runtime:{rt}"), format!("profile:{prof}")],
    };
    store
        .put_module_record(KIND_PLUGIN, &rec.id, &rec)
        .map_err(|e| e.to_string())?;
    Ok(rec)
}

fn parse_plugin_mcp_server(plugin_id: &str, raw: &Value) -> Result<McpServerConfig, String> {
    if let Ok(mut cfg) = serde_json::from_value::<McpServerConfig>(raw.clone()) {
        if cfg.id.trim().is_empty() {
            cfg.id = format!("plugin:{plugin_id}:{}", cfg.name);
        }
        return Ok(cfg);
    }
    let name = raw
        .get("name")
        .or_else(|| raw.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("plugin '{plugin_id}' MCP entry missing name/id"))?
        .to_string();
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("plugin:{plugin_id}:{name}"));
    let transport = match raw
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "streamable_http" | "http" => McpTransport::StreamableHttp,
        "server_sent_events" | "sse" => McpTransport::ServerSentEvents,
        _ if raw.get("url").and_then(|v| v.as_str()).is_some() => McpTransport::StreamableHttp,
        _ => McpTransport::Stdio,
    };
    let command = raw
        .get("command")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let args = raw
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let env = raw
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let url = raw
        .get("url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let headers = raw
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerConfig {
        id,
        name,
        transport,
        enabled: raw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        command,
        args,
        env,
        url,
        headers,
        oauth_client_id: raw
            .get("oauth_client_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        oauth_scopes: raw
            .get("oauth_scopes")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Reconcile enabled plugins from `<workspace>/.nola/plugins/*/nola-plugin.json` and `Store`,
/// loading their declared MCP server configs, skill directories, and custom page entries.
pub fn reconcile_enabled_plugins(store: &Store, workspace_root: &Path) -> PluginRuntimeSummary {
    let mut summary = PluginRuntimeSummary::default();
    let stored: BTreeMap<String, PluginRecord> = store
        .list_module_records::<PluginRecord>(KIND_PLUGIN)
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect();

    let plugins_dir = workspace_root.join(".nola/plugins");
    let Ok(entries) = fs::read_dir(&plugins_dir) else {
        return summary;
    };

    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let manifest_path = dir.join("nola-plugin.json");
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let manifest: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                summary.errors.push(format!(
                    "failed to parse {}: {e}",
                    manifest_path.display()
                ));
                continue;
            }
        };
        let folder_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin")
            .to_string();
        let id = manifest
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&folder_name)
            .to_string();
        let enabled = stored
            .get(&id)
            .map(|r| r.enabled)
            .unwrap_or_else(|| manifest.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true));
        if !enabled {
            continue;
        }

        let name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();
        let version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0")
            .to_string();
        summary.enabled_plugins.push(json!({
            "id": id,
            "name": name,
            "version": version,
            "directory": dir.display().to_string()
        }));

        // 1. Declared MCP servers (`mcp_servers` or `mcpServers`)
        if let Some(mcp_val) = manifest
            .get("mcp_servers")
            .or_else(|| manifest.get("mcpServers"))
        {
            match mcp_val {
                Value::Array(arr) => {
                    for entry in arr {
                        match parse_plugin_mcp_server(&id, entry) {
                            Ok(cfg) if cfg.enabled => summary.mcp_servers.push(cfg),
                            Ok(_) => {}
                            Err(e) => summary.errors.push(e),
                        }
                    }
                }
                Value::Object(map) => {
                    for (key, val) in map {
                        let mut merged = val.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            obj.entry("name").or_insert_with(|| json!(key));
                        }
                        match parse_plugin_mcp_server(&id, &merged) {
                            Ok(cfg) if cfg.enabled => summary.mcp_servers.push(cfg),
                            Ok(_) => {}
                            Err(e) => summary.errors.push(e),
                        }
                    }
                }
                _ => {}
            }
        }

        // 2. Declared skill directories (`skill_dirs` / `skills`) or implicit `skills/`
        let mut declared_skill_paths: Vec<String> = manifest
            .get("skill_dirs")
            .or_else(|| manifest.get("skills"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if declared_skill_paths.is_empty() && dir.join("skills").is_dir() {
            declared_skill_paths.push("skills".into());
        }
        for rel in declared_skill_paths {
            let candidate = dir.join(&rel);
            if candidate.is_dir()
                && path_within_workspace(&dir, &candidate)
                && !summary.skill_dirs.contains(&candidate)
            {
                summary.skill_dirs.push(candidate);
            }
        }

        // 3. Custom pages (`pages.json` or manifest `pages`)
        let raw_pages: Vec<Value> = if let Ok(raw_p) = fs::read_to_string(dir.join("pages.json")) {
            match serde_json::from_str::<Value>(&raw_p) {
                Ok(Value::Array(arr)) => arr,
                Ok(Value::Object(map)) => map
                    .get("pages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        } else {
            manifest
                .get("pages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        };
        for item in raw_pages {
            let Some(pid) = item
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            else {
                continue;
            };
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(pid);
            let html = if let Some(inline) = item.get("html").and_then(|v| v.as_str()) {
                inline.to_string()
            } else if let Some(rel_file) = item
                .get("html_path")
                .or_else(|| item.get("file"))
                .and_then(|v| v.as_str())
            {
                let candidate = dir.join(rel_file);
                if path_within_workspace(&dir, &candidate) {
                    fs::read_to_string(&candidate)
                        .ok()
                        .filter(|s| s.len() <= MAX_PLUGIN_HTML_BYTES)
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            summary.pages.push(json!({
                "id": pid,
                "title": title,
                "plugin_id": id,
                "package": folder_name,
                "html": html
            }));
        }
    }

    summary
}

/// Scan `<workspace>/.nola/plugins/*/nola-plugin.json`, merge persisted `PluginRecord`s,
/// probe `pi`/`dsh` CLI availability on `PATH`, and return `{profiles, plugins}` matching
/// `HarnessPlugins.svelte`.
pub fn list_harness_plugins(store: Option<&Store>, workspace_root: &Path) -> Value {
    let pi_available = find_binary_on_path("pi").is_some();
    let dsh_available = find_binary_on_path("dsh").is_some();

    let mut discovered_plugins: Vec<Value> = Vec::new();
    let mut pi_packages: Vec<Value> = Vec::new();
    let mut dsh_web_packages: Vec<Value> = Vec::new();
    let mut dsh_term_packages: Vec<Value> = Vec::new();

    let plugins_dir = workspace_root.join(".nola/plugins");
    if let Ok(entries) = fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join("nola-plugin.json");
            let Ok(raw) = fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let folder_name = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("plugin")
                .to_string();
            let id = manifest
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&folder_name)
                .to_string();
            let name = manifest
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&id)
                .to_string();
            let version = manifest
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.1.0")
                .to_string();
            let runtime = manifest
                .get("runtime")
                .and_then(|v| v.as_str())
                .unwrap_or("dsh");
            let profile = manifest
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("web");
            let source = manifest
                .get("source")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{name}@{version}"));

            let pages: Vec<Value> = manifest
                .get("pages")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let pid = p.get("id").and_then(|v| v.as_str())?;
                            let title = p.get("title").and_then(|v| v.as_str()).unwrap_or(pid);
                            Some(json!({ "id": pid, "title": title }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let links: Vec<Value> = manifest
                .get("links")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| {
                            let url = l.get("url").and_then(|v| v.as_str())?;
                            let title = l.get("title").and_then(|v| v.as_str()).unwrap_or(url);
                            Some(json!({ "title": title, "url": url }))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let pkg_entry = json!({
                "source": source,
                "pages": pages,
                "links": links
            });
            match (runtime, profile) {
                ("pi", _) => pi_packages.push(pkg_entry),
                (_, "terminal") => dsh_term_packages.push(pkg_entry),
                _ => dsh_web_packages.push(pkg_entry),
            }

            discovered_plugins.push(json!({
                "id": id,
                "name": name,
                "version": version,
                "entry_path": manifest.get("entry_path").and_then(|v| v.as_str()).unwrap_or(""),
                "manifest_path": manifest_path.display().to_string(),
                "enabled": manifest.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                "capabilities": manifest.get("capabilities").cloned().unwrap_or_else(|| json!([])),
                "source": "manifest"
            }));
        }
    }

    if let Some(st) = store {
        for rec in st.list_module_records::<PluginRecord>(KIND_PLUGIN).unwrap_or_default() {
            if !discovered_plugins
                .iter()
                .any(|p| p.get("id").and_then(|v| v.as_str()) == Some(rec.id.as_str()))
            {
                let src = format!("{}@{}", rec.name, rec.version);
                let pkg_entry = json!({
                    "source": src,
                    "pages": [],
                    "links": []
                });
                let has_cap = |needle: &str| {
                    rec.capabilities
                        .iter()
                        .any(|c| c.trim().eq_ignore_ascii_case(needle))
                };
                if has_cap("pi") || has_cap("runtime:pi") {
                    pi_packages.push(pkg_entry);
                } else if has_cap("terminal")
                    || has_cap("dsh_terminal")
                    || has_cap("profile:terminal")
                {
                    dsh_term_packages.push(pkg_entry);
                } else {
                    dsh_web_packages.push(pkg_entry);
                }
                discovered_plugins.push(json!({
                    "id": rec.id,
                    "name": rec.name,
                    "version": rec.version,
                    "entry_path": rec.entry_path,
                    "enabled": rec.enabled,
                    "capabilities": rec.capabilities,
                    "source": "store"
                }));
            }
        }
    }

    json!({
        "profiles": [
            {
                "runtime": "pi",
                "profile": "pi",
                "available": pi_available,
                "packages": pi_packages
            },
            {
                "runtime": "dsh",
                "profile": "web",
                "available": dsh_available,
                "packages": dsh_web_packages
            },
            {
                "runtime": "dsh",
                "profile": "terminal",
                "available": dsh_available,
                "packages": dsh_term_packages
            }
        ],
        "plugins": discovered_plugins
    })
}

/// Scan `<workspace>/.nola/plugins/*/pages.json` (and `nola-plugin.json` `pages`) to build
/// the structured desktop plugin page catalog matching `DesktopPlugin.svelte`.
pub fn list_desktop_plugin_pages(workspace_root: &Path) -> Vec<Value> {
    let mut pages = Vec::new();
    let plugins_dir = workspace_root.join(".nola/plugins");
    let Ok(entries) = fs::read_dir(&plugins_dir) else {
        return pages;
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let pkg_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin")
            .to_string();

        let raw_pages: Vec<Value> = if let Ok(raw) = fs::read_to_string(dir.join("pages.json")) {
            match serde_json::from_str::<Value>(&raw) {
                Ok(Value::Array(arr)) => arr,
                Ok(Value::Object(map)) => map
                    .get("pages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        } else if let Ok(raw) = fs::read_to_string(dir.join("nola-plugin.json")) {
            serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|v| v.get("pages").and_then(|p| p.as_array()).cloned())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        for item in raw_pages {
            let Some(id) = item
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            else {
                continue;
            };
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(id);
            let html = if let Some(inline) = item.get("html").and_then(|v| v.as_str()) {
                inline.to_string()
            } else if let Some(rel_file) = item
                .get("html_path")
                .or_else(|| item.get("file"))
                .and_then(|v| v.as_str())
            {
                let candidate = dir.join(rel_file);
                if path_within_workspace(&dir, &candidate) {
                    fs::read_to_string(&candidate)
                        .ok()
                        .filter(|s| s.len() <= MAX_PLUGIN_HTML_BYTES)
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            pages.push(json!({
                "id": id,
                "title": title,
                "package": item.get("package").and_then(|v| v.as_str()).unwrap_or(&pkg_name),
                "html": html
            }));
        }
    }

    pages.sort_by(|a, b| {
        a.get("id")
            .and_then(|v| v.as_str())
            .cmp(&b.get("id").and_then(|v| v.as_str()))
    });
    pages
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(LocalModelManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(PluginManageTool {
        suite: Arc::clone(suite),
    }));
}

struct LocalModelManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for LocalModelManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "local_model_manage".into(),
                description: "Register, list, probe hardware/engines, search HuggingFace hub, download/resume, run setup, recommend, start/stop, or delete local LLM engines (llama.cpp/Ollama/vLLM/MLX/LM Studio).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "register",
                                "list",
                                "probe",
                                "hardware",
                                "engine_status",
                                "recommend",
                                "search_hub",
                                "setup",
                                "start",
                                "stop",
                                "running",
                                "download_start",
                                "download_status",
                                "download_cancel",
                                "delete"
                            ]
                        },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "engine": { "type": "string" },
                        "endpoint": { "type": "string" },
                        "model_path": { "type": "string" },
                        "path": { "type": "string" },
                        "port": { "type": "integer" },
                        "extra_args": { "type": "array", "items": { "type": "string" } },
                        "query": { "type": "string" },
                        "mirror_base": { "type": "string" },
                        "url": { "type": "string" },
                        "dest_path": { "type": "string" },
                        "sha256": { "type": "string" },
                        "quantization": { "type": "string" },
                        "download_id": { "type": "string" },
                        "opt_in_install": { "type": "boolean" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "register" => {
                let rec = LocalModelRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("local-model")
                        .to_string(),
                    engine: args
                        .get("engine")
                        .and_then(|v| v.as_str())
                        .unwrap_or("llama.cpp")
                        .to_string(),
                    path: args.get("path").and_then(|v| v.as_str()).map(ToString::to_string),
                    size_bytes: args.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                    quantization: args
                        .get("quantization")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    installed: true,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_LOCAL_MODEL, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Registered local model '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<LocalModelRecord>(KIND_LOCAL_MODEL) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "models": items}),
                        format!("Listed {count} local model(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
            },
            "hardware" => {
                let hw = probe_hardware();
                ToolExecutionResult::ok_structured(hw, "Probed host hardware")
            }
            "engine_status" => {
                let engine = args
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("llama.cpp");
                let st = probe_engine_status(engine);
                let msg = st
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Engine probed")
                    .to_string();
                ToolExecutionResult::ok_structured(st, msg)
            }
            "setup" => {
                let engine = args
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("llama.cpp");
                let opt_in = args
                    .get("opt_in_install")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let res = run_engine_setup_with_opt_in(engine, opt_in);
                let msg = res
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Engine setup evaluated")
                    .to_string();
                ToolExecutionResult::ok_structured(res, msg)
            }
            "recommend" => {
                let hw = probe_hardware();
                let recs = recommend_local_models(&hw);
                ToolExecutionResult::ok_structured(recs, "Computed local model recommendations")
            }
            "search_hub" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let engine = args
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("llama.cpp");
                let mirror = args.get("mirror_base").and_then(|v| v.as_str());
                match search_hub_models(query, engine, mirror) {
                    Ok(models) => {
                        let count = models.len();
                        ToolExecutionResult::ok_structured(
                            json!({"query": query, "engine": engine, "count": count, "models": models}),
                            format!("Found {count} hub model candidate(s)"),
                        )
                    }
                    Err(e) => ToolExecutionResult::err_structured("hub_search_error", e, None),
                }
            }
            "start" => {
                let id_arg = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let stored = (!id_arg.is_empty())
                    .then(|| self.suite.get_record::<LocalModelRecord>(KIND_LOCAL_MODEL, id_arg).ok().flatten())
                    .flatten();
                let engine = args
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| stored.as_ref().map(|r| r.engine.clone()))
                    .unwrap_or_else(|| "llama.cpp".into());
                let model_path = args
                    .get("model_path")
                    .or_else(|| args.get("path"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| stored.as_ref().and_then(|r| r.path.clone()))
                    .or_else(|| stored.as_ref().map(|r| r.name.clone()))
                    .unwrap_or_default();
                let port = args
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(8081);
                let extra_args: Vec<String> = args
                    .get("extra_args")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                match LocalModelProcessManager::global().start_local_model(
                    id_arg,
                    &engine,
                    &model_path,
                    port,
                    &extra_args,
                ) {
                    Ok(started) => ToolExecutionResult::ok_structured(
                        started,
                        format!("Started local model server ({engine}) on port {port}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("start_failed", e, None),
                }
            }
            "stop" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match LocalModelProcessManager::global().stop_local_model(id) {
                    Ok(res) => ToolExecutionResult::ok_structured(
                        res,
                        format!("Stop requested for local model '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("stop_failed", e, None),
                }
            }
            "download_start" => {
                if !ctx.network_allowed {
                    return ToolExecutionResult::err_structured(
                        "network_denied",
                        "model download blocked by network policy",
                        None,
                    );
                }
                let Some(url) = args.get("url").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'url'", None);
                };
                let Some(dest_path) = args
                    .get("dest_path")
                    .or_else(|| args.get("path"))
                    .and_then(|v| v.as_str())
                else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'dest_path'", None);
                };
                let plan = ModelDownloadPlan {
                    id: args.get("download_id").and_then(|v| v.as_str()).map(ToString::to_string),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("local-model")
                        .to_string(),
                    engine: args
                        .get("engine")
                        .and_then(|v| v.as_str())
                        .unwrap_or("llama.cpp")
                        .to_string(),
                    url: url.to_string(),
                    dest_path: dest_path.to_string(),
                    sha256: args.get("sha256").and_then(|v| v.as_str()).map(ToString::to_string),
                    quantization: args
                        .get("quantization")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    expected_bytes: args.get("expected_bytes").and_then(|v| v.as_u64()),
                };
                match ModelDownloadManager::global().start_download(&self.suite.store, plan) {
                    Ok(st) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&st).unwrap_or(json!({})),
                        format!("Started model download '{}'", st.download_id),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("download_error", e, None),
                }
            }
            "download_status" => {
                let st = ModelDownloadManager::global().current_status(&self.suite.store);
                ToolExecutionResult::ok_structured(st, "Fetched model download status")
            }
            "download_cancel" => {
                let Some(dl_id) = args
                    .get("download_id")
                    .or_else(|| args.get("id"))
                    .and_then(|v| v.as_str())
                else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'download_id'", None);
                };
                match ModelDownloadManager::global().cancel_download(&self.suite.store, dl_id) {
                    Ok(res) => ToolExecutionResult::ok_structured(
                        res,
                        format!("Cancelled model download '{dl_id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("download_error", e, None),
                }
            }
            "running" => {
                let running = LocalModelProcessManager::global().list_running_models();
                let count = running.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "running": running}),
                    format!("{count} local model server(s) running"),
                )
            }
            "probe" => {
                let endpoint = args
                    .get("endpoint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("http://127.0.0.1:8081");
                let raw_url = format!("{}/v1/models", endpoint.trim_end_matches('/'));
                let parsed = match Url::parse(&raw_url) {
                    Ok(u) if matches!(u.scheme(), "http" | "https") => u,
                    _ => {
                        return ToolExecutionResult::err_structured(
                            "invalid_endpoint",
                            "endpoint must be a valid http(s) URL",
                            None,
                        );
                    }
                };
                let is_loopback = match parsed.host() {
                    Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                    Some(Host::Ipv4(v4)) => v4.is_loopback(),
                    Some(Host::Ipv6(v6)) => v6.is_loopback(),
                    None => false,
                };
                if !is_loopback {
                    if !ctx.network_allowed {
                        return ToolExecutionResult::err_structured(
                            "network_denied",
                            "non-loopback endpoint probe blocked by network policy",
                            None,
                        );
                    }
                    if let Err(e) = nola_tools::web::resolve_and_validate_public_url(&raw_url) {
                        return ToolExecutionResult::err_structured("ssrf_blocked", e, None);
                    }
                }
                let res = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .and_then(|c| c.get(parsed).send());
                match res {
                    Ok(resp) if resp.status().is_success() => {
                        let mut buf = Vec::new();
                        let _ = resp.take(MAX_PROBE_BODY_BYTES).read_to_end(&mut buf);
                        let body: Value = serde_json::from_slice(&buf).unwrap_or(json!({}));
                        ToolExecutionResult::ok_structured(
                            json!({"endpoint": endpoint, "reachable": true, "models": body}),
                            format!("Probed local endpoint {endpoint}"),
                        )
                    }
                    Ok(resp) => ToolExecutionResult::err_structured(
                        "probe_failed",
                        format!("{raw_url} returned HTTP {}", resp.status()),
                        None,
                    ),
                    Err(e) => ToolExecutionResult::err_structured("probe_error", e.to_string(), None),
                }
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_LOCAL_MODEL, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted local model '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported local_model action '{other}'"),
                None,
            ),
        }
    }
}

struct PluginManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for PluginManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "plugin_manage".into(),
                description: "Register, install, reconcile, list, inspect harness catalog/pages, or remove Harness Plugins.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["register", "install", "reconcile", "list", "catalog", "pages", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "version": { "type": "string" },
                        "entry_path": { "type": "string" },
                        "runtime": { "type": "string" },
                        "profile": { "type": "string" },
                        "source": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "register" => {
                let rec = PluginRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("plugin")
                        .to_string(),
                    version: args
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.1.0")
                        .to_string(),
                    entry_path: args
                        .get("entry_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    enabled: true,
                    capabilities: Vec::new(),
                };
                match self.suite.upsert_record(KIND_PLUGIN, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Registered plugin '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
                }
            }
            "install" => {
                let Some(source) = args.get("source").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'source'", None);
                };
                let runtime = args.get("runtime").and_then(|v| v.as_str()).unwrap_or("dsh");
                let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("web");
                match install_plugin_package(
                    &self.suite.store,
                    &self.suite.workspace_root,
                    runtime,
                    profile,
                    source,
                ) {
                    Ok(rec) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Installed plugin '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("plugin_install_error", e, None),
                }
            }
            "reconcile" => {
                let summary =
                    reconcile_enabled_plugins(&self.suite.store, &self.suite.workspace_root);
                let count = summary.enabled_plugins.len();
                ToolExecutionResult::ok_structured(
                    serde_json::to_value(&summary).unwrap_or(json!({})),
                    format!("Reconciled {count} enabled plugin(s)"),
                )
            }
            "list" => match self.suite.list_records::<PluginRecord>(KIND_PLUGIN) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "plugins": items}),
                        format!("Listed {count} plugin(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
            },
            "catalog" => {
                let catalog =
                    list_harness_plugins(Some(&self.suite.store), &self.suite.workspace_root);
                ToolExecutionResult::ok_structured(catalog, "Loaded harness plugin catalog")
            }
            "pages" => {
                let pages = list_desktop_plugin_pages(&self.suite.workspace_root);
                let count = pages.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "pages": pages}),
                    format!("Discovered {count} desktop plugin page(s)"),
                )
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_PLUGIN, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted plugin '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported plugin action '{other}'"),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_and_engine_probes_return_truthful_results() {
        let hw = probe_hardware();
        assert_eq!(hw["os"], std::env::consts::OS);
        assert_eq!(hw["arch"], std::env::consts::ARCH);
        assert!(hw["cpu_cores"].as_u64().unwrap() >= 1);
        #[cfg(target_os = "linux")]
        {
            assert!(hw["memoryBytes"].as_u64().unwrap() > 0);
            assert!(hw["freeDiskBytes"].as_u64().unwrap() > 0);
        }

        // Unknown engine is never faked as ready
        let bogus = probe_engine_status("nonexistent-engine-xyz");
        assert_eq!(bogus["ready"], false);
        assert!(bogus["binaryPath"].is_null());

        // Non-existent override path for llama-server must report ready: false
        let prev = std::env::var("NOLA_LLAMA_SERVER").ok();
        std::env::set_var("NOLA_LLAMA_SERVER", "/definitely/missing/llama-server");
        let st = probe_engine_status("llama.cpp");
        if st["binaryPath"].is_null() {
            assert_eq!(st["ready"], false);
        }
        if let Some(v) = prev {
            std::env::set_var("NOLA_LLAMA_SERVER", v);
        } else {
            std::env::remove_var("NOLA_LLAMA_SERVER");
        }

        // Recommendations scale with hardware
        let recs = recommend_local_models(&hw);
        assert!(recs["candidates"].as_array().is_some_and(|a| !a.is_empty()));
    }

    #[test]
    fn harness_plugins_and_desktop_pages_scan_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join(".nola/plugins/metrics-ui");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("nola-plugin.json"),
            r#"{"id":"metrics-ui","name":"Metrics UI","version":"1.2.0","pages":[{"id":"metrics","title":"Metrics"}]}"#,
        )
        .unwrap();
        fs::write(
            plugin_dir.join("pages.json"),
            r#"[{"id":"metrics","title":"Metrics","html":"<h1>Live Metrics</h1>"}]"#,
        )
        .unwrap();

        let catalog = list_harness_plugins(None, dir.path());
        assert_eq!(catalog["profiles"].as_array().unwrap().len(), 3);
        assert_eq!(catalog["plugins"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["plugins"][0]["id"], "metrics-ui");

        let pages = list_desktop_plugin_pages(dir.path());
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["id"], "metrics");
        assert_eq!(pages[0]["html"], "<h1>Live Metrics</h1>");

        // Stored PluginRecords route to pi / dsh-terminal / dsh-web by capability
        let store = Store::open_in_memory().unwrap();
        store
            .put_module_record(
                KIND_PLUGIN,
                "p-pi",
                &PluginRecord {
                    id: "p-pi".into(),
                    name: "pi-tool".into(),
                    version: "1.0.0".into(),
                    entry_path: "".into(),
                    enabled: true,
                    capabilities: vec!["pi".into()],
                },
            )
            .unwrap();
        store
            .put_module_record(
                KIND_PLUGIN,
                "p-term",
                &PluginRecord {
                    id: "p-term".into(),
                    name: "term-tool".into(),
                    version: "1.0.0".into(),
                    entry_path: "".into(),
                    enabled: true,
                    capabilities: vec!["terminal".into()],
                },
            )
            .unwrap();
        let cat2 = list_harness_plugins(Some(&store), dir.path());
        let profiles = cat2["profiles"].as_array().unwrap();
        assert_eq!(profiles[0]["packages"].as_array().unwrap().len(), 1);
        assert_eq!(profiles[2]["packages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn lmstudio_probe_hub_search_fallback_setup_resumable_downloader_and_plugin_runtime_work() {
        // 1. LM Studio probe & setup (truthful when missing)
        let lms_st = probe_engine_status("lmstudio");
        assert_eq!(lms_st["engine"], "lmstudio");
        assert_eq!(lms_st["canInstall"], true);
        let setup_missing = run_engine_setup_with_opt_in("nonexistent-engine-xyz", false);
        assert_eq!(setup_missing["ready"], false);
        assert!(!setup_missing["steps"].as_array().unwrap().is_empty());

        // 2. Hub search falls back cleanly to curated models when mirror is unreachable
        let hub_results =
            search_hub_models("qwen", "llama.cpp", Some("http://127.0.0.1:1")).unwrap();
        assert!(!hub_results.is_empty());
        assert_eq!(hub_results[0]["offlineFallback"], true);

        // 3. Real resumable background ModelDownloadManager with Range + SHA-256 verification
        let payload = b"GGUF_FAKE_MODEL_HEADER_0123456789_PAYLOAD_END".to_vec();
        let expected_sha = sha256_hex(&payload);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_payload = payload.clone();
        let server_thread = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 2048];
                let n = stream.read(&mut req_buf).unwrap_or(0);
                let req_str = String::from_utf8_lossy(&req_buf[..n]);
                let range_start = req_str
                    .lines()
                    .find_map(|l| {
                        let lower = l.to_ascii_lowercase();
                        lower.strip_prefix("range: bytes=").and_then(|rest| {
                            rest.split('-').next().and_then(|s| s.trim().parse::<usize>().ok())
                        })
                    })
                    .unwrap_or(0);
                let slice = &server_payload[range_start.min(server_payload.len())..];
                let header = if range_start > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        range_start,
                        server_payload.len() - 1,
                        server_payload.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    )
                };
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(slice);
                let _ = stream.flush();
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let dest_path = dir.path().join("models/test.gguf");
        fs::create_dir_all(dest_path.parent().unwrap()).unwrap();
        // Pre-seed first 10 bytes in .part to verify Range resume!
        let part_path = PathBuf::from(format!("{}.part", dest_path.display()));
        fs::write(&part_path, &payload[..10]).unwrap();

        let mgr: &'static ModelDownloadManager = Box::leak(Box::new(ModelDownloadManager::new()));
        let started = mgr
            .start_download(
                &store,
                ModelDownloadPlan {
                    id: Some("dl-resume-1".into()),
                    name: "test-gguf".into(),
                    engine: "llama.cpp".into(),
                    url: format!("http://{addr}/test.gguf"),
                    dest_path: dest_path.display().to_string(),
                    sha256: Some(expected_sha.clone()),
                    quantization: Some("Q4_K_M".into()),
                    expected_bytes: Some(payload.len() as u64),
                },
            )
            .unwrap();
        assert_eq!(started.download_id, "dl-resume-1");

        let mut final_status = json!({});
        for _ in 0..50 {
            final_status = mgr.current_status(&store);
            if matches!(
                final_status["status"].as_str(),
                Some("installed") | Some("completed") | Some("failed") | Some("cancelled")
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        let _ = server_thread.join();
        assert_eq!(final_status["status"], "installed", "status: {final_status}");
        assert_eq!(final_status["id"], "dl-resume-1");
        assert_eq!(final_status["repository"], "test-gguf");
        assert_eq!(final_status["source"], "huggingface");
        assert_eq!(final_status["sha256"], expected_sha);
        assert_eq!(fs::read(&dest_path).unwrap(), payload);
        let models: Vec<LocalModelRecord> = store.list_module_records(KIND_LOCAL_MODEL).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "test-gguf");

        // 4. Plugin install (with SHA-256 verification) & runtime reconciliation (MCP + skills + pages)
        let src_plugin = dir.path().join("src-plugin");
        fs::create_dir_all(src_plugin.join("skills/demo")).unwrap();
        fs::write(src_plugin.join("entry.js"), "console.log('hi');").unwrap();
        let entry_sha = sha256_hex(b"console.log('hi');");
        fs::write(
            src_plugin.join("skills/demo/SKILL.md"),
            "---\nname: plugin-skill\ndescription: From plugin\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            src_plugin.join("nola-plugin.json"),
            serde_json::to_string(&json!({
                "id": "acme-kit",
                "name": "Acme Kit",
                "version": "2.0.0",
                "entry_path": "entry.js",
                "sha256": entry_sha,
                "mcp_servers": [
                    { "name": "acme-mcp", "transport": "stdio", "command": "echo", "args": ["ok"], "enabled": true }
                ],
                "skill_dirs": ["skills"],
                "pages": [
                    { "id": "acme-dash", "title": "Acme Dashboard", "html": "<p>Acme</p>" }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let installed = install_plugin_package(
            &store,
            dir.path(),
            "dsh",
            "web",
            &src_plugin.display().to_string(),
        )
        .unwrap();
        assert_eq!(installed.id, "acme-kit");

        let reconciled = reconcile_enabled_plugins(&store, dir.path());
        assert_eq!(reconciled.enabled_plugins.len(), 1);
        assert_eq!(reconciled.mcp_servers.len(), 1);
        assert_eq!(reconciled.mcp_servers[0].name, "acme-mcp");
        assert_eq!(reconciled.skill_dirs.len(), 1);
        assert_eq!(reconciled.pages.len(), 1);
        assert_eq!(reconciled.pages[0]["id"], "acme-dash");
    }

    #[test]
    fn ssrf_redirect_dest_confinement_plugin_traversal_and_engine_reason_codes() {
        // 1. Structured engineStatus.reason codes
        let unsupported = probe_engine_status("nonexistent-engine-xyz");
        assert_eq!(unsupported["reason"], "unsupported");
        let prev = std::env::var("NOLA_LLAMA_SERVER").ok();
        std::env::set_var("NOLA_LLAMA_SERVER", "/definitely/missing/llama-server");
        let overridden = probe_engine_status("llama.cpp");
        if overridden["ready"] == false {
            assert_eq!(overridden["reason"], "invalid_override");
        }
        if let Some(v) = prev {
            std::env::set_var("NOLA_LLAMA_SERVER", v);
        } else {
            std::env::remove_var("NOLA_LLAMA_SERVER");
        }

        // 2. Direct redirect target validator blocks HTTP downgrade & private/loopback/metadata IPs
        let base = Url::parse("https://huggingface.co/owner/repo/resolve/main/model.gguf").unwrap();
        assert!(validate_download_redirect_target(&base, "http://huggingface.co/model.gguf").is_err());
        assert!(validate_download_redirect_target(&base, "https://127.0.0.1/secret").is_err());
        assert!(validate_download_redirect_target(&base, "https://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_download_redirect_target(&base, "http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_download_redirect_target(&base, "file:///etc/passwd").is_err());

        // 3. End-to-end download blocks SSRF 302 redirect to metadata IP
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let resp = "HTTP/1.1 302 Found\r\nLocation: https://169.254.169.254/latest/meta-data\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let dest_path = dir.path().join("redirected.gguf");
        let mgr: &'static ModelDownloadManager = Box::leak(Box::new(ModelDownloadManager::new()));
        mgr.start_download(
            &store,
            ModelDownloadPlan {
                id: Some("dl-ssrf-redir".into()),
                name: "ssrf-test".into(),
                engine: "llama.cpp".into(),
                url: format!("http://{addr}/model.gguf"),
                dest_path: dest_path.display().to_string(),
                sha256: None,
                quantization: None,
                expected_bytes: None,
            },
        )
        .unwrap();
        let mut final_status = json!({});
        for _ in 0..50 {
            final_status = mgr.current_status(&store);
            if matches!(final_status["status"].as_str(), Some("failed") | Some("installed")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let _ = server.join();
        assert_eq!(final_status["status"], "failed", "SSRF redirect must fail: {final_status}");
        assert!(
            final_status["error"]
                .as_str()
                .unwrap_or("")
                .contains("SSRF"),
            "expected SSRF error, got: {final_status}"
        );
        assert!(!dest_path.exists());

        // 4. dest_path confinement rejects .., sensitive paths, and symlink escapes
        assert!(safe_model_dest_path(dir.path(), "../escape.gguf").is_err());
        assert!(safe_model_dest_path(dir.path(), "/etc/passwd").is_err());
        assert!(safe_model_dest_path(dir.path(), ".ssh/authorized_keys").is_err());
        assert!(safe_model_dest_path(dir.path(), "vault.json").is_err());
        assert!(safe_model_dest_path(dir.path(), "nested/ok.gguf").is_ok());
        assert!(mgr
            .start_download(
                &store,
                ModelDownloadPlan {
                    id: None,
                    name: "bad".into(),
                    engine: "llama.cpp".into(),
                    url: "http://127.0.0.1:9/a.gguf".into(),
                    dest_path: dir.path().join("../escape.gguf").display().to_string(),
                    sha256: None,
                    quantization: None,
                    expected_bytes: None,
                },
            )
            .is_err());
        assert!(mgr
            .start_download(
                &store,
                ModelDownloadPlan {
                    id: None,
                    name: "bad-ssh".into(),
                    engine: "llama.cpp".into(),
                    url: "http://127.0.0.1:9/a.gguf".into(),
                    dest_path: dir.path().join(".ssh/authorized_keys").display().to_string(),
                    sha256: None,
                    quantization: None,
                    expected_bytes: None,
                },
            )
            .is_err());

        // 5. Plugin entry_path traversal, symlink, and -flag CLI injection rejection
        let bad_trav = dir.path().join("bad-plugin-trav");
        fs::create_dir_all(&bad_trav).unwrap();
        fs::write(
            bad_trav.join("nola-plugin.json"),
            r#"{"id":"bad-trav","name":"Bad","entry_path":"../../etc/passwd"}"#,
        )
        .unwrap();
        let trav_err = install_plugin_package(
            &store,
            dir.path(),
            "dsh",
            "web",
            &bad_trav.display().to_string(),
        )
        .unwrap_err();
        assert!(trav_err.contains("entry_path"), "got: {trav_err}");

        #[cfg(unix)]
        {
            let outside_secret = dir.path().join("outside.js");
            fs::write(&outside_secret, "secret").unwrap();
            let bad_sym = dir.path().join("bad-plugin-sym");
            fs::create_dir_all(&bad_sym).unwrap();
            std::os::unix::fs::symlink(&outside_secret, bad_sym.join("entry.js")).unwrap();
            fs::write(
                bad_sym.join("nola-plugin.json"),
                r#"{"id":"bad-sym","name":"BadSym","entry_path":"entry.js"}"#,
            )
            .unwrap();
            let sym_err = install_plugin_package(
                &store,
                dir.path(),
                "dsh",
                "web",
                &bad_sym.display().to_string(),
            )
            .unwrap_err();
            assert!(sym_err.contains("symlink"), "got: {sym_err}");
        }

        let flag_err = install_plugin_package(
            &store,
            dir.path(),
            "dsh",
            "web",
            "--malicious-flag=1",
        )
        .unwrap_err();
        assert!(
            flag_err.contains("must not start with '-'"),
            "expected flag rejection, got: {flag_err}"
        );
    }
}
