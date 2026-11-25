// SPDX-License-Identifier: Apache-2.0
//! Local Models hardware probe, engine runtime process manager, recommendations,
//! and Harness Plugins / Desktop Plugin Pages catalog.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nola_store::Store;
use nola_tools::{path_within_workspace, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, LocalModelRecord, PluginRecord, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};
use url::{Host, Url};

use crate::ModulesSuite;

const MAX_PROBE_BODY_BYTES: u64 = 256 * 1024;
const MAX_PLUGIN_HTML_BYTES: usize = 512 * 1024;

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

    let ready = binary_path.is_some();
    let backend = if canonical_engine == "llama.cpp" {
        backend_env.unwrap_or_else(|| default_backend.to_string())
    } else {
        default_backend.to_string()
    };
    let can_install = matches!(canonical_engine, "llama.cpp" | "ollama" | "vllm");
    let message = match &binary_path {
        Some(p) => format!("{canonical_engine} ready at {}", p.display()),
        None if bin_candidates.is_empty() => format!("unsupported local model engine '{engine}'"),
        None => format!(
            "{canonical_engine} binary ({}) not found on PATH",
            bin_candidates.join(" / ")
        ),
    };

    json!({
        "engine": canonical_engine,
        "ready": ready,
        "binaryPath": binary_path.map(|p| p.display().to_string()),
        "backend": backend,
        "canInstall": can_install,
        "guideUrl": guide_url,
        "reason": message,
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
                description: "Register, list, probe hardware/engines, recommend, start/stop, or delete local LLM engines (llama.cpp/Ollama/vLLM/MLX).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["register", "list", "probe", "hardware", "engine_status", "recommend", "running", "delete"]
                        },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "engine": { "type": "string" },
                        "endpoint": { "type": "string" }
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
            "recommend" => {
                let hw = probe_hardware();
                let recs = recommend_local_models(&hw);
                ToolExecutionResult::ok_structured(recs, "Computed local model recommendations")
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
                description: "Register, list, inspect harness catalog/pages, or remove Harness Plugins.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["register", "list", "catalog", "pages", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "version": { "type": "string" },
                        "entry_path": { "type": "string" }
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
}
