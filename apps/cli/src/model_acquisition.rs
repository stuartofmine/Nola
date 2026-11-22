// SPDX-License-Identifier: Apache-2.0
//! Desktop model acquisition shared by the native helper, CLI and Web Host.
mod cache;
mod device;
mod engine_setup;
pub(crate) use engine_setup::{engine_status, setup_engine};
mod hub;
mod modelfit;
mod planning;
mod selection;
mod storage;
mod vllm_checkpoint;
use vllm_checkpoint::Backend as VllmBackend;

use hub::Hub;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Llama,
    Vllm,
    Mlx,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    HuggingFace,
    ModelScope,
}
impl Engine {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "llama.cpp" => Ok(Self::Llama),
            "vllm" => Ok(Self::Vllm),
            "mlx" => Ok(Self::Mlx),
            _ => Err("Choose llama.cpp, vllm or mlx".into()),
        }
    }
    fn for_model(s: &str) -> Result<Self, String> {
        let engine = Self::parse(s)?;
        if engine == Self::Vllm {
            VllmBackend::current()?;
        }
        Ok(engine)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Llama => "llama.cpp",
            Self::Vllm => "vllm",
            Self::Mlx => "mlx",
        }
    }
    fn matches_metadata(self, repo: &str, tags: &Value) -> bool {
        let tagged = |word: &str| {
            tags.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|v| {
                    v.eq_ignore_ascii_case(word)
                        || v.rsplit(':')
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case(word))
                })
        };
        let name = repo.to_ascii_lowercase();
        let mlx = name.contains("mlx") || tagged("mlx");
        let gguf = name.contains("gguf") || tagged("gguf");
        match self {
            Self::Llama => gguf,
            Self::Mlx => mlx,
            Self::Vllm => VllmBackend::current().is_ok_and(|b| b.accepts_repo(mlx, gguf)),
        }
    }
}
impl Source {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "huggingface" => Ok(Self::HuggingFace),
            "modelscope" => Ok(Self::ModelScope),
            _ => Err("Choose Hugging Face or ModelScope".into()),
        }
    }
    fn parse_or_default(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("auto")
            || trimmed.eq_ignore_ascii_case("default")
        {
            Ok(Self::default_for_environment())
        } else {
            Self::parse(trimmed)
        }
    }
    fn default_for_environment() -> Self {
        let env_override = std::env::var("NOLA_MODEL_SOURCE").ok();
        let tz_offset_secs = Some(chrono::Local::now().offset().local_minus_utc());
        let tz_name = detect_timezone_name();
        let locale = detect_locale();
        default_source_for(
            env_override.as_deref(),
            tz_offset_secs,
            tz_name.as_deref(),
            locale.as_deref(),
        )
    }
    fn name(self) -> &'static str {
        match self {
            Self::HuggingFace => "huggingface",
            Self::ModelScope => "modelscope",
        }
    }
}

pub(crate) fn default_source_name() -> &'static str {
    Source::default_for_environment().name()
}

fn detect_timezone_name() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        let trimmed = tz.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    #[cfg(unix)]
    {
        if let Ok(target) = fs::read_link("/etc/localtime") {
            let text = target.to_string_lossy();
            if let Some((_, zone)) = text.split_once("zoneinfo/") {
                if !zone.is_empty() {
                    return Some(zone.to_owned());
                }
            }
        }
    }
    None
}

fn detect_locale() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("C") && !trimmed.eq_ignore_ascii_case("POSIX") {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn default_source_for(
    env_override: Option<&str>,
    tz_offset_secs: Option<i32>,
    tz_name: Option<&str>,
    locale: Option<&str>,
) -> Source {
    if let Some(raw) = env_override.map(str::trim).filter(|s| !s.is_empty()) {
        if raw.eq_ignore_ascii_case("modelscope") {
            return Source::ModelScope;
        }
        if raw.eq_ignore_ascii_case("huggingface") {
            return Source::HuggingFace;
        }
    }
    if let Some(tz) = tz_name {
        let lower = tz.trim().to_ascii_lowercase();
        let is_china_tz = matches!(
            lower.as_str(),
            "asia/shanghai"
                | "asia/chongqing"
                | "asia/harbin"
                | "asia/urumqi"
                | "asia/beijing"
                | "prc"
                | "cst-8"
        ) || lower.contains("shanghai")
            || lower.contains("chongqing")
            || lower.contains("harbin")
            || lower.contains("urumqi");
        if is_china_tz {
            return Source::ModelScope;
        }
    }
    if tz_offset_secs == Some(8 * 3600) {
        if let Some(loc) = locale {
            let lower = loc.trim().to_ascii_lowercase().replace('_', "-");
            if lower.starts_with("zh") || lower.contains("zh-cn") || lower.contains("zh-hans") {
                return Source::ModelScope;
            }
        }
    }
    Source::HuggingFace
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileSpec {
    path: String,
    bytes: u64,
    revision: String,
    sha256: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    revision: String,
    files: Vec<FileSpec>,
    context: Option<u64>,
    tags: Value,
}

pub(crate) use storage::{model_lock, read_catalog, update_catalog};

fn root_path() -> Result<PathBuf, String> {
    match std::env::var_os("NOLA_MODELS_ROOT") {
        Some(root) if Path::new(&root).is_absolute() => Ok(PathBuf::from(root)),
        Some(_) => Err("NOLA_MODELS_ROOT must be an absolute directory".into()),
        None => crate::local_models::models_root(),
    }
}
pub(crate) fn hardware() -> Value {
    json!({
        "hardware": device::snapshot(),
        "defaultSource": Source::default_for_environment().name()
    })
}
pub(crate) use planning::{install_plan, recommend};

pub(crate) fn validate_vllm_directory(directory: &Path) -> Result<(), String> {
    let cfg = directory.join("config.json");
    if storage::plain_file_len(&cfg)?.is_none_or(|size| size > 4 * 1024 * 1024) {
        return Err("Missing or oversized local vLLM model configuration".into());
    }
    VllmBackend::current()?.checkpoint(&storage::read_json(&cfg)?)?;
    Ok(())
}
pub(crate) fn search(engine: &str, source: &str, query: &str) -> Result<Value, String> {
    let query = query.trim();
    let requested_source = Source::parse_or_default(source)?;
    let engine = Engine::for_model(engine)?;
    if let Some(direct) = planning::parse_direct_input(query) {
        let effective_source = direct.source.unwrap_or(requested_source);
        let mut res = Hub::new(effective_source)?.search(engine, &direct.repository)?;
        if let Some(obj) = res.as_object_mut() {
            obj.insert("resolvedSource".into(), json!(effective_source.name()));
            obj.insert("sourceAutoDetected".into(), json!(direct.source.is_some()));
            obj.insert("resolvedRepository".into(), json!(direct.repository));
        }
        return Ok(res);
    }
    if query.is_empty()
        || query.len() > 255
        || query.chars().any(char::is_control)
        || (query.contains('/') && !hub::valid_repository(query))
    {
        return Err("Enter a model name, owner/repository, or Hugging Face / ModelScope URL".into());
    }
    let mut res = Hub::new(requested_source)?.search(engine, query)?;
    if let Some(obj) = res.as_object_mut() {
        obj.insert("resolvedSource".into(), json!(requested_source.name()));
        obj.insert("sourceAutoDetected".into(), json!(false));
    }
    Ok(res)
}

fn resolved_files(
    hub: &Hub,
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
) -> Result<Vec<FileSpec>, String> {
    resolved_files_for_budget_with_file(
        hub,
        engine,
        repo,
        manifest,
        crate::local_models::physical_memory_bytes().map(|m| m / 4 * 3),
        None,
    )
}
fn resolved_files_for_budget(
    hub: &Hub,
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
    budget: Option<u64>,
) -> Result<Vec<FileSpec>, String> {
    resolved_files_for_budget_with_file(hub, engine, repo, manifest, budget, None)
}
fn resolved_files_for_budget_with_file(
    hub: &Hub,
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
    budget: Option<u64>,
    preferred_file: Option<&str>,
) -> Result<Vec<FileSpec>, String> {
    let mut files =
        selection::primary_for_budget_with_file(engine, repo, manifest, budget, preferred_file)?;
    if engine == Engine::Vllm {
        let cfg = files
            .iter()
            .find(|f| f.path == "config.json")
            .ok_or("Missing model configuration")?;
        VllmBackend::current()?.checkpoint(&hub.configuration(repo, cfg)?)?;
    }
    if let Some(index) = files
        .iter()
        .find(|f| f.path == "model.safetensors.index.json")
    {
        if index.bytes > 64 * 1024 * 1024 {
            return Err("Safetensors index is too large".into());
        }
        let response = hub.artifact(repo, index, 0)?;
        let mut data = Vec::new();
        response
            .take(index.bytes + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() as u64 != index.bytes {
            return Err("Truncated safetensors index".into());
        }
        let index = serde_json::from_slice(&data).map_err(|_| "Invalid safetensors index")?;
        selection::apply_index(&mut files, &index)?;
    }
    Ok(files)
}
pub(crate) fn install(
    engine: &str,
    source: &str,
    repo: &str,
    progress: impl FnMut(u64, u64) -> bool,
) -> Result<Value, String> {
    let engine = Engine::for_model(engine)?;
    let source = Source::parse(source)?;
    let hub = Hub::new(source)?;
    let manifest = hub.manifest(repo)?;
    let files = resolved_files(&hub, engine, repo, &manifest)?;
    activate(engine, source, repo, &hub, manifest, files, None, progress)
}

fn model_id(engine: Engine, source: Source, repo: &str) -> Result<String, String> {
    let (owner, name) = repo.split_once('/').ok_or("Invalid model repository")?;
    Ok(format!(
        "hub-{}-{}-{}-{}-{}",
        engine.name(),
        source.name(),
        owner.len(),
        owner,
        name
    ))
}

/// Keep existing revision-only snapshots readable. Other quantizations in that
/// immutable revision have a stable identifier based on their sorted manifest position.
fn snapshot_names(
    model_dir: &Path,
    engine: Engine,
    manifest: &Manifest,
    files: &[FileSpec],
) -> Result<(String, String), String> {
    let rev = &manifest.revision;
    let legacy_partial = format!(".partial-{rev}");
    if engine != Engine::Llama {
        return Ok((rev.clone(), legacy_partial));
    }
    let first = &files.first().ok_or("Missing GGUF artifact")?.path;
    let position = manifest
        .files
        .iter()
        .filter(|f| f.path.as_str() < first.as_str())
        .count();
    let variant = format!("{rev}-gguf-{position}");
    let snapshots = model_dir.join("snapshots");
    let mut active = rev.clone();
    if storage::plain_dir_exists(&snapshots)? {
        let base = snapshots.join(rev);
        if storage::plain_dir_exists(&base)? {
            let original = storage::read_json(&base.join("snapshot.json"))?;
            let artifacts = original["artifacts"]
                .as_array()
                .ok_or("Invalid installed snapshot manifest")?;
            if !artifacts.iter().any(|f| f["path"] == *first) {
                active = variant.clone();
            }
        }
    }
    let mut staging = format!(".partial-{variant}");
    let old = model_dir.join(&legacy_partial);
    if storage::plain_dir_exists(&old)? && storage::completed_bytes(&old, files)? > 0 {
        staging = legacy_partial;
    }
    Ok((active, staging))
}

fn activate(
    engine: Engine,
    source: Source,
    repo: &str,
    hub: &Hub,
    manifest: Manifest,
    files: Vec<FileSpec>,
    plan: Option<&Value>,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<Value, String> {
    let total = files
        .iter()
        .try_fold(0u64, |n, f| n.checked_add(f.bytes))
        .ok_or("Invalid model size")?;
    if !progress(0, total) {
        return Err("Model download cancelled".into());
    }
    let root = root_path()?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let (_, name) = repo.split_once('/').ok_or("Invalid model repository")?;
    let id = model_id(engine, source, repo)?;
    let model_dir = storage::directory(&root, &id)?;
    let _install_lock = storage::model_lock(&model_dir)?;
    let snapshots = storage::directory(&model_dir, "snapshots")?;
    let (active, partial) = snapshot_names(&model_dir, engine, &manifest, &files)?;
    let snapshot = snapshots.join(active);
    let existing = storage::plain_dir_exists(&snapshot)?;
    let staging = if existing {
        snapshot.clone()
    } else {
        storage::directory(&model_dir, &partial)?
    };
    if !existing {
        storage::check_space(&staging, &files, total)?;
    }
    let mut complete = 0;
    for file in &files {
        let path = storage::artifact_path(&staging, &file.path)?;
        if existing {
            if storage::plain_file_len(&path)? != Some(file.bytes) {
                return Err(format!(
                    "Installed artifact is missing or changed: {}",
                    file.path
                ));
            }
        } else {
            storage::download(hub, repo, file, &path, |current| {
                progress(complete + current, total)
            })?;
        }
        complete += file.bytes;
        if !progress(complete, total) {
            return Err("Model download cancelled".into());
        }
    }
    let context = if engine == Engine::Llama {
        selection::gguf_context(&staging.join(&files[0].path))?.or(manifest.context)
    } else {
        let cfg = storage::read_json(&staging.join("config.json"))?;
        cfg.pointer("/text_config/max_position_embeddings")
            .or(cfg.get("max_position_embeddings"))
            .or(cfg.get("n_positions"))
            .and_then(Value::as_u64)
    }
    .filter(|&n| n > 0);
    let artifacts = files
        .iter()
        .map(|f| json!({"path":f.path,"bytes":f.bytes}))
        .collect::<Vec<_>>();
    if !existing {
        storage::write_json(
            &staging.join("snapshot.json"),
            &json!({"schemaVersion":1,"repository":repo,"revision":manifest.revision,"totalBytes":total,"artifacts":artifacts}),
        )?;
        fs::rename(&staging, &snapshot)
            .map_err(|e| format!("Cannot activate model snapshot: {e}"))?;
    }
    let entry = if engine == Engine::Llama {
        &files[0].path
    } else {
        "config.json"
    };
    let mut record = json!({"id":id,"name":name,"repository":repo,"source":source.name(),"revision":manifest.revision,
        "runtime":engine.name(),"format": match engine{Engine::Llama=>"gguf",Engine::Vllm=>"safetensors",Engine::Mlx=>"mlx"},
        "contextWindowTokens":context.unwrap_or(4096),"contextWindowReported":context.is_some(),"artifactPath":snapshot.join(entry),"snapshotPath":snapshot,
        "artifactBytes":total,"artifacts":artifacts,"state":"configured","enabled":true});
    if let Some(plan) = plan {
        let target = plan["contextTokens"]
            .as_u64()
            .ok_or("Missing recommended context")?;
        record["estimatedPeakMemoryBytes"] = plan["estimatedMemoryBytes"].clone();
        record["recommendedDevice"] = plan["device"].clone();
        record["recommendedContextTokens"] = json!(context.map_or(target, |max| target.min(max)));
    }
    update_catalog(&root, |models| {
        models.retain(|m| m["id"] != id);
        models.push(record.clone());
        Ok(())
    })?;
    Ok(json!({"model":record}))
}

pub(crate) fn cli(args: &[String]) -> Option<Result<(), String>> {
    if args.first().map(String::as_str) != Some("model")
        || args.get(1).map(String::as_str) != Some("acquire")
    {
        return None;
    }
    Some((|| {
        if let [_, _, action] = args {
            if action == "hardware" {
                println!("{}", hardware());
                return Ok(());
            }
        }
        if let [_, _, action, engine] = args {
            if action == "recommend" {
                println!(
                    "{}",
                    recommend(engine, Source::default_for_environment().name(), "")?
                );
                return Ok(());
            }
        }
        if let [_, _, action, engine, source] = args {
            if action == "recommend" {
                println!("{}", recommend(engine, source, "")?);
                return Ok(());
            }
        }
        if let [_, _, action, id] = args {
            if action == "engine-status" || action == "setup-engine" {
                let result = if action == "engine-status" {
                    engine_status(id, true)?
                } else {
                    setup_engine(id)?
                };
                println!("{result}");
                return Ok(());
            }
            if action == "remove" {
                crate::local_models::remove_installed_snapshot(id)?;
                println!("{}", json!({"removed":true}));
                return Ok(());
            }
        }
        let [_, _, action, engine, source, value] = args else {
            return Err("Usage: nola model acquire hardware | recommend|search|confirm|install <llama.cpp|vllm|mlx> <huggingface|modelscope> <model name or repository>".into());
        };
        let mut out = io::stdout().lock();
        let result = match action.as_str() {
            "search" => search(engine, source, value)?,
            "recommend" => recommend(engine, source, value)?,
            "install" | "confirm" => {
                let mut last = None;
                let progress = |done, total| {
                    if last.is_none_or(|t: Instant| t.elapsed().as_millis() >= 250) || done == total
                    {
                        last = Some(Instant::now());
                        writeln!(
                            out,
                            "{}",
                            json!({"downloadedBytes":done,"totalBytes":total})
                        )
                        .and_then(|_| out.flush())
                        .is_ok()
                    } else {
                        true
                    }
                };
                if action == "confirm" {
                    let plan =
                        serde_json::from_str(value).map_err(|_| "Invalid installation plan")?;
                    install_plan(engine, source, &plan, progress)?
                } else {
                    install(engine, source, value, progress)?
                }
            }
            _ => return Err("Unknown model acquisition operation".into()),
        };
        writeln!(out, "{result}").map_err(|e| e.to_string())
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_source_prefers_modelscope_for_china_utc_plus_8_and_respects_override() {
        // Explicit env override always wins
        assert_eq!(
            default_source_for(Some("huggingface"), Some(8 * 3600), Some("Asia/Shanghai"), Some("zh_CN.UTF-8")),
            Source::HuggingFace
        );
        assert_eq!(
            default_source_for(Some("modelscope"), Some(-8 * 3600), Some("America/Los_Angeles"), Some("en_US.UTF-8")),
            Source::ModelScope
        );
        // China IANA timezones default to ModelScope even with en_US locale
        for tz in ["Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin", "Asia/Urumqi", "PRC"] {
            assert_eq!(
                default_source_for(None, Some(8 * 3600), Some(tz), Some("en_US.UTF-8")),
                Source::ModelScope,
                "expected ModelScope for {tz}"
            );
        }
        // UTC+8 with Chinese locale (e.g. Windows without IANA TZ name) defaults to ModelScope
        assert_eq!(
            default_source_for(None, Some(8 * 3600), None, Some("zh_CN.UTF-8")),
            Source::ModelScope
        );
        assert_eq!(
            default_source_for(None, Some(8 * 3600), None, Some("zh-Hans")),
            Source::ModelScope
        );
        // Non-China UTC+8 with English locale stays HuggingFace
        assert_eq!(
            default_source_for(None, Some(8 * 3600), Some("Asia/Singapore"), Some("en_US.UTF-8")),
            Source::HuggingFace
        );
        // Other timezones stay HuggingFace
        assert_eq!(
            default_source_for(None, Some(0), Some("UTC"), Some("en_US.UTF-8")),
            Source::HuggingFace
        );
        assert_eq!(
            default_source_for(None, Some(-5 * 3600), Some("America/New_York"), Some("zh_CN.UTF-8")),
            Source::HuggingFace
        );
    }
}
