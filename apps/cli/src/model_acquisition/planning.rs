// SPDX-License-Identifier: Apache-2.0
use super::*;
use reqwest::Url;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[path = "context.rs"]
mod context;
use context::Architecture;
pub(super) const TARGET_CONTEXT: u64 = 131_072;
const RECOMMEND_DEADLINE: Duration = Duration::from_secs(8);

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const CHANGED:&str="The repository, selected files or runtime settings changed. Request a new recommendation before downloading.";

pub(super) fn adaptive_target_context(memory_bytes: u64) -> u64 {
    if memory_bytes == 0 || memory_bytes <= 8 * GIB {
        8_192
    } else if memory_bytes <= 16 * GIB {
        32_768
    } else if memory_bytes <= 32 * GIB {
        65_536
    } else {
        TARGET_CONTEXT
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DirectTarget {
    pub source: Option<Source>,
    pub repository: String,
    pub filename: Option<String>,
}

pub(super) fn parse_direct_input(input: &str) -> Option<DirectTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 512 || trimmed.chars().any(char::is_control) {
        return None;
    }
    // Direct owner/repo input
    if !trimmed.contains("://")
        && !trimmed.starts_with("huggingface.co/")
        && !trimmed.starts_with("www.huggingface.co/")
        && !trimmed.starts_with("hf.co/")
        && !trimmed.starts_with("hf-mirror.com/")
        && !trimmed.starts_with("modelscope.cn/")
        && !trimmed.starts_with("www.modelscope.cn/")
    {
        if hub::valid_repository(trimmed) {
            return Some(DirectTarget {
                source: None,
                repository: trimmed.to_owned(),
                filename: None,
            });
        }
        return None;
    }
    let candidate_url = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = Url::parse(&candidate_url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segments: Vec<&str> = parsed
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    if matches!(
        host.as_str(),
        "huggingface.co" | "www.huggingface.co" | "hf.co" | "hf-mirror.com" | "www.hf-mirror.com"
    ) {
        if segments.len() < 2 {
            return None;
        }
        // Skip non-model prefixes like /datasets/ or /spaces/
        if matches!(segments[0], "datasets" | "spaces" | "docs" | "models") {
            return None;
        }
        let repo = format!("{}/{}", segments[0], segments[1]);
        if !hub::valid_repository(&repo) {
            return None;
        }
        let filename = if segments.len() >= 5 && matches!(segments[2], "blob" | "resolve") {
            let file = segments[4..].join("/");
            (file.to_ascii_lowercase().ends_with(".gguf") && selection::safe_path(&file))
                .then_some(file)
        } else {
            None
        };
        return Some(DirectTarget {
            source: Some(Source::HuggingFace),
            repository: repo,
            filename,
        });
    }
    if matches!(host.as_str(), "modelscope.cn" | "www.modelscope.cn") {
        // Expected: /models/{owner}/{repo}[/files | /summary | /file/view/{rev}/{file} | /resolve/{rev}/{file}]
        if segments.len() < 3 || segments[0] != "models" {
            return None;
        }
        let repo = format!("{}/{}", segments[1], segments[2]);
        if !hub::valid_repository(&repo) {
            return None;
        }
        let mut filename = if segments.len() >= 7
            && segments[3] == "file"
            && segments[4] == "view"
        {
            let file = segments[6..].join("/");
            (file.to_ascii_lowercase().ends_with(".gguf") && selection::safe_path(&file))
                .then_some(file)
        } else if segments.len() >= 6 && matches!(segments[3], "resolve" | "blob") {
            let file = segments[5..].join("/");
            (file.to_ascii_lowercase().ends_with(".gguf") && selection::safe_path(&file))
                .then_some(file)
        } else {
            None
        };
        if filename.is_none() {
            filename = parsed
                .query_pairs()
                .find(|(k, _)| k.eq_ignore_ascii_case("filepath") || k.eq_ignore_ascii_case("file"))
                .map(|(_, v)| v.into_owned())
                .filter(|f| f.to_ascii_lowercase().ends_with(".gguf") && selection::safe_path(f));
        }
        return Some(DirectTarget {
            source: Some(Source::ModelScope),
            repository: repo,
            filename,
        });
    }
    None
}

struct Capacity {
    memory: u64,
    disk: u64,
    device: &'static str,
    target_context: u64,
}
impl Capacity {
    fn for_engine(engine: Engine, hw: &Value) -> Result<Self, String> {
        let ram = hw["memoryBytes"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("Cannot determine system memory")?;
        let disk = hw["freeDiskBytes"]
            .as_u64()
            .ok_or("Cannot determine free space at the model cache")?;
        let unified = hw["unifiedMemory"].as_bool() == Some(true);
        let vram = hw["gpuMemoryBytes"].as_u64().filter(|n| *n > 0);
        // Leave room for the OS, display and other applications. Never add shared GPU memory to RAM.
        let (memory,device)=match engine {
            Engine::Mlx if !unified=>return Err("MLX requires an Apple silicon Mac".into()),
            _ if unified=>(ram / 3*2,"unified"),
            Engine::Llama if vram.is_some()=>(vram.unwrap_or_default()/5*4,"gpu"),
            Engine::Vllm=>match vram {
                Some(v)=>(v/5*4,"gpu"),
                None=>return Err("The host could not verify dedicated GPU memory for vLLM. Choose llama.cpp for CPU inference.".into())
            },
            _=>(ram/5*3,"cpu"),
        };
        Ok(Self {
            memory,
            disk,
            device,
            target_context: adaptive_target_context(ram),
        })
    }
    fn weights(&self) -> u64 {
        self.memory.saturating_sub(512 * MIB + 128 * MIB) / 11 * 10
    }
}
fn json_metadata(hub: &Hub, repo: &str, manifest: &Manifest, name: &str) -> Result<Value, String> {
    let Some(file) = manifest.files.iter().find(|f| f.path == name) else {
        return Ok(Value::Null);
    };
    hub.configuration(repo, file)
}
fn estimate_with_arch(
    cap: &Capacity,
    bytes: u64,
    completed: u64,
    arch: &Architecture,
) -> Result<(u64, u64), String> {
    if storage::space_required(bytes, completed) > cap.disk {
        return Err("Insufficient free disk space for the model snapshot".into());
    }
    let fixed = bytes
        .saturating_add(bytes / 10)
        .saturating_add(arch.static_bytes());
    let available = cap.memory.saturating_sub(fixed) / arch.bytes_per_token();
    let context = arch
        .windows()
        .find(|n| *n <= available && *n > 0)
        .ok_or("Estimated model and context memory exceed this device's budget")?;
    Ok((
        context,
        fixed.saturating_add(arch.bytes_per_token().saturating_mul(context)),
    ))
}

#[cfg(test)]
fn estimate(
    cap: &Capacity,
    bytes: u64,
    completed: u64,
    cfg: &Value,
    native: Option<u64>,
) -> Result<(u64, u64), String> {
    estimate_with_arch(cap, bytes, completed, &Architecture::detect(cfg, native))
}

fn architecture(
    hub: &Hub,
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
    config: &Value,
) -> (Architecture, Option<String>, Option<String>) {
    let local = Architecture::detect(config, manifest.context);
    if engine == Engine::Llama && (local.native.is_none() || local.kv.is_none()) {
        // An inaccessible, ambiguous or gated upstream does not license a guessed
        // architecture. Keep the short-window plan based on local evidence.
        let upstream = hub
            .quantization_base(repo, manifest)
            .ok()
            .flatten()
            .and_then(|base| {
                let source = hub.manifest(&base).ok()?;
                let cfg = json_metadata(hub, &base, &source, "config.json").ok()?;
                let arch = Architecture::detect(&cfg, local.native);
                (arch.kv.is_some() && arch.native.is_some()).then_some((
                    arch,
                    Some(base),
                    Some(source.revision),
                ))
            });
        if let Some(upstream) = upstream {
            return upstream;
        }
    }
    let source = (local.native.is_some() || local.kv.is_some()).then(|| repo.to_owned());
    let revision = source.as_ref().map(|_| manifest.revision.clone());
    (local, source, revision)
}

fn weights_quantized(config: &Value) -> bool {
    let is_quant = |v: &Value| {
        v.get("quant_method")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            || v["bits"].as_u64().is_some_and(|n| n > 0 && n <= 8)
    };
    config.get("quantization_config").is_some_and(is_quant)
        || config.get("quantization").is_some_and(is_quant)
        || config
            .get("dtype")
            .or(config.get("torch_dtype"))
            .and_then(Value::as_str)
            .is_some_and(|s| {
                matches!(
                    s.to_ascii_lowercase().as_str(),
                    "int4" | "uint4" | "int8" | "uint8" | "float8_e4m3fn" | "float8_e5m2" | "fp8"
                )
            })
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedPreview {
    manifest: Manifest,
    files: Vec<FileSpec>,
    plan: Value,
}

fn preview_cache_key(
    source: Source,
    engine: Engine,
    repo: &str,
    revision: Option<&str>,
    variant: Option<&str>,
) -> String {
    match (revision, variant) {
        (Some(rev), Some(var)) => format!("{}|{}|{}|{}|{}", source.name(), engine.name(), repo, rev, var),
        (Some(rev), None) => format!("{}|{}|{}|{}", source.name(), engine.name(), repo, rev),
        _ => format!("{}|{}|{}", source.name(), engine.name(), repo),
    }
}

struct Preview {
    manifest: Manifest,
    files: Vec<FileSpec>,
    plan: Value,
    candidate: Value,
}
fn preview(
    hub: &Hub,
    engine: Engine,
    source: Source,
    repo: &str,
    cap: &Capacity,
) -> Result<Preview, String> {
    preview_with_file(hub, engine, source, repo, None, cap)
}

fn preview_with_file(
    hub: &Hub,
    engine: Engine,
    source: Source,
    repo: &str,
    preferred_file: Option<&str>,
    cap: &Capacity,
) -> Result<Preview, String> {
    let manifest = hub.manifest(repo)?;
    let config = json_metadata(hub, repo, &manifest, "config.json")?;
    let (arch, metadata_source, metadata_revision) =
        architecture(hub, engine, repo, &manifest, &config);
    let model_dir = root_path()?.join(model_id(engine, source, repo)?);
    // Existing caches can contribute all or part of a variant. The exact
    // snapshot is checked below. New installs must fit the whole transfer.
    let disk_limit = if storage::plain_dir_exists(&model_dir)? {
        u64::MAX
    } else {
        cap.disk.saturating_sub(512 * MIB).min(cap.disk / 21 * 20)
    };
    let budget = if engine == Engine::Llama && arch.native.is_some() && arch.kv.is_some() {
        selection::context_gguf_budget(
            &manifest.files,
            arch.windows()
                .map(|w| arch.weight_limit(cap.memory, w).min(disk_limit)),
            cap.weights().min(disk_limit),
        )
    } else {
        cap.weights().min(disk_limit)
    };
    let files = resolved_files_for_budget_with_file(
        hub,
        engine,
        repo,
        &manifest,
        Some(budget),
        preferred_file,
    )?;
    let bytes = files
        .iter()
        .try_fold(0u64, |n, f| n.checked_add(f.bytes))
        .ok_or("Invalid model size")?;
    // Only lightweight metadata and weight indexes are fetched by the preview.
    let mut completed = 0;
    if storage::plain_dir_exists(&model_dir)? {
        let snapshots = model_dir.join("snapshots");
        let (active, partial) = snapshot_names(&model_dir, engine, &manifest, &files)?;
        let active = snapshots.join(active);
        let partial = model_dir.join(partial);
        if storage::plain_dir_exists(&snapshots)? && storage::plain_dir_exists(&active)? {
            completed = storage::completed_bytes(&active, &files)?;
        } else if storage::plain_dir_exists(&partial)? {
            completed = storage::completed_bytes(&partial, &files)?;
        }
    }
    let (context, memory) = estimate_with_arch(cap, bytes, completed, &arch)?;
    let native_context_known = arch.native.is_some();
    let quantized = if engine == Engine::Llama {
        selection::gguf_quantized(&files[0].path)
    } else {
        weights_quantized(&config)
    };
    let artifacts: Vec<_> = files
        .iter()
        .map(|f| json!({"path":f.path,"bytes":f.bytes,"revision":f.revision}))
        .collect();
    let format_label = config
        .pointer("/quantization_config/quant_method")
        .and_then(Value::as_str)
        .or_else(|| {
            config
                .get("dtype")
                .or(config.get("torch_dtype"))
                .and_then(Value::as_str)
        })
        .unwrap_or("safetensors");
    let variant = match engine {
        Engine::Llama => files[0].path.clone(),
        Engine::Vllm => VllmBackend::current()?
            .checkpoint(&config)?
            .unwrap_or_else(|| format_label.to_owned()),
        Engine::Mlx => format_label.to_owned(),
    };
    let target_ctx = cap.target_context;
    let reason = arch.reason_for_target(context, target_ctx);
    let mut plan = json!({"schemaVersion":1,"engine":engine.name(),"source":source.name(),"repository":repo,
        "revision":manifest.revision,"files":artifacts,"variant":variant,"downloadBytes":bytes,
        "contextTokens":context,"targetContextTokens":target_ctx,"contextLimitReason":reason,
        "nativeContextTokens":arch.native,"kvBytesPerToken":arch.kv,"contextMetadataSource":metadata_source,"contextMetadataRevision":metadata_revision,
        "quantized":quantized,"nativeContextKnown":native_context_known,"estimatedMemoryBytes":memory,"device":cap.device});
    if engine == Engine::Vllm {
        match VllmBackend::current()? {
            VllmBackend::MetalMlx => plan["runtimeBackend"] = json!("metal-mlx"),
            VllmBackend::MetalMps => plan["runtimeBackend"] = json!("metal-mps"),
            VllmBackend::Standard => {}
        }
    }
    let candidate = json!({"repository":repo,"source":source.name(),"revision":manifest.revision,"variant":variant,"downloadBytes":bytes,
        "contextTokens":context,"targetContextTokens":target_ctx,"contextLimitReason":reason,
        "nativeContextTokens":arch.native,"kvBytesPerToken":arch.kv,"contextMetadataSource":metadata_source,"contextMetadataRevision":metadata_revision,
        "quantized":quantized,"nativeContextKnown":native_context_known,"estimatedMemoryBytes":memory,"device":cap.device,"plan":plan});
    let cached = CachedPreview {
        manifest: manifest.clone(),
        files: files.clone(),
        plan: plan.clone(),
    };
    let specific_key = preview_cache_key(
        source,
        engine,
        repo,
        Some(manifest.revision.as_str()),
        Some(variant.as_str()),
    );
    cache::set("preview", &specific_key, &cached, cache::PREVIEW_TTL);
    let generic_key = preview_cache_key(source, engine, repo, None, None);
    cache::set("preview", &generic_key, &cached, cache::PREVIEW_TTL);
    Ok(Preview {
        manifest,
        files,
        plan,
        candidate,
    })
}
fn family_matches(repo: &str, query: &str) -> bool {
    let normalize = |name: &str| {
        name.to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    };
    let model = normalize(repo.split_once('/').map_or(repo, |(_, n)| n));
    let wanted = normalize(query.split_once('/').map_or(query, |(_, n)| n));
    if model == wanted {
        return true;
    }
    let Some(tail) = model.strip_prefix(&format!("{wanted}-")) else {
        return false;
    };
    tail.split('-').all(|t| {
        matches!(
            t,
            "instruct"
                | "it"
                | "chat"
                | "gguf"
                | "mlx"
                | "bit"
                | "awq"
                | "ggml"
                | "fp16"
                | "bf16"
                | "gptq"
                | "int4"
                | "int8"
                | "4bit"
                | "8bit"
                | "4"
                | "8"
                | "k"
                | "m"
                | "s"
                | "f16"
                | "q4"
                | "q5"
                | "q6"
                | "q8"
        )
    })
}
fn has_chat_template(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(items) => items
            .iter()
            .any(|x| x.get("template").is_some_and(has_chat_template)),
        Value::Object(items) => items
            .values()
            .any(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())),
        _ => false,
    }
}

fn conversational(hub: &Hub, repo: &str, manifest: &Manifest) -> Result<bool, String> {
    if manifest.files.iter().any(|f| {
        f.bytes > 0
            && (f.path == "chat_template.jinja"
                || (f.path.starts_with("chat_templates/") && f.path.ends_with(".jinja")))
    }) || manifest
        .tags
        .as_array()
        .is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some("conversational")))
    {
        return Ok(true);
    }
    let cfg = json_metadata(hub, repo, manifest, "tokenizer_config.json")?;
    Ok(has_chat_template(&cfg["chat_template"]))
}
pub(super) fn unsloth_repository(repo: &str) -> bool {
    repo.split_once('/')
        .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case("unsloth"))
}

pub(super) fn target_met(candidate: &Value) -> bool {
    let target = candidate["targetContextTokens"]
        .as_u64()
        .unwrap_or(TARGET_CONTEXT);
    candidate["contextLimitReason"] == "target_met"
        && candidate["contextTokens"]
            .as_u64()
            .is_some_and(|n| n >= target)
}

fn preferred_quantization(item: &Value) -> bool {
    item["repository"].as_str().is_some_and(unsloth_repository) && item["quantized"] == true
}

pub(super) fn rank_candidates(candidates: &mut [Value]) {
    // Preserve the upstream recommendation order on ties. A short-window conversion
    // cannot displace a runnable long-window model just because of its publisher.
    candidates.sort_by_key(|item| {
        (
            std::cmp::Reverse(target_met(item)),
            std::cmp::Reverse(item["contextTokens"].as_u64().unwrap_or(0)),
            !preferred_quantization(item),
        )
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lookup {
    Search,
    Family,
    PreferredFamily,
}

fn collect(
    hub: &Hub,
    engine: Engine,
    source: Source,
    query: &str,
    cap: &Capacity,
    mode: Lookup,
) -> Result<Vec<Value>, String> {
    let strict_family = mode != Lookup::Search;
    let exact = hub::valid_repository(query);
    // Publishers sometimes omit "Instruct" from an instruction-tuned model's name.
    // An alternate spelling is admissible only when its repository also proves chat support.
    let discovery_query = if strict_family {
        query.strip_suffix("-Instruct").unwrap_or(query)
    } else {
        query
    };
    let inspect = |repo: &str| {
        let needs_chat_evidence = strict_family && !family_matches(repo, query);
        if needs_chat_evidence && !family_matches(repo, discovery_query) {
            return Ok(None);
        }
        match preview(hub, engine, source, repo, cap) {
            Ok(v) => {
                if (needs_chat_evidence
                    || engine != Engine::Llama && (strict_family || query.is_empty()))
                    && !conversational(hub, repo, &v.manifest)?
                {
                    // GGUF snapshots may carry the chat template only inside the weights.
                    // The declared source of this conversion can supply independent evidence.
                    let base = v.candidate["contextMetadataSource"]
                        .as_str()
                        .filter(|base| *base != repo && engine == Engine::Llama);
                    let supports_chat = base.is_some_and(|base| {
                        hub.manifest(base)
                            .and_then(|m| conversational(hub, base, &m))
                            .unwrap_or(false)
                    });
                    if !supports_chat {
                        return Ok(None);
                    }
                }
                Ok(Some(v.candidate))
            }
            Err(e) if exact => Err(e),
            Err(e) if e.contains("request failed") || e.contains("error sending request") => Err(e),
            Err(_) => Ok(None),
        }
    };
    if mode == Lookup::PreferredFamily {
        for repo in hub
            .preferred_repositories(engine, discovery_query)?
            .into_iter()
            .take(6)
        {
            if let Ok(Some(candidate)) = inspect(&repo) {
                if target_met(&candidate) && preferred_quantization(&candidate) {
                    return Ok(vec![candidate]);
                }
            }
        }
        return Ok(Vec::new());
    }
    hub.find(
        engine,
        discovery_query,
        if strict_family { 1 } else { 4 },
        true,
        inspect,
    )
}

fn catalog_candidates(
    queries: &[String],
    engine: Engine,
    lookup: impl Fn(&str, Lookup) -> Option<Value> + Sync,
) -> Vec<Value> {
    catalog_candidates_with_deadline(queries, engine, None, lookup)
}

fn catalog_candidates_with_deadline(
    queries: &[String],
    engine: Engine,
    deadline: Option<Instant>,
    lookup: impl Fn(&str, Lookup) -> Option<Value> + Sync,
) -> Vec<Value> {
    let queries = &queries[..queries.len().min(8)];
    let mut matches = vec![None; queries.len()];
    'passes: for mode in [Lookup::PreferredFamily, Lookup::Family] {
        if mode == Lookup::PreferredFamily && engine != Engine::Llama {
            continue;
        }
        // A verified long-window Unsloth quantization already has the highest ranking.
        // Look for these across the shortlist before inspecting other publishers.
        let width = if mode == Lookup::PreferredFamily {
            4
        } else {
            2
        };
        for start in (0..queries.len()).step_by(width) {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                break 'passes;
            }
            let end = (start + width).min(queries.len());
            std::thread::scope(|scope| {
                let mut workers = Vec::new();
                for index in start..end {
                    if matches[index].is_none() {
                        let query = &queries[index];
                        let lookup = &lookup;
                        workers.push((index, scope.spawn(move || lookup(query, mode))));
                    }
                }
                for (index, worker) in workers {
                    matches[index] = worker.join().ok().flatten();
                }
            });
            let distinct_targets: HashSet<_> = matches
                .iter()
                .flatten()
                .filter(|v| target_met(v))
                .filter_map(|v| v["repository"].as_str())
                .collect();
            if distinct_targets.len() >= 3 {
                break 'passes;
            }
        }
    }
    let mut seen = HashSet::new();
    matches
        .into_iter()
        .flatten()
        .filter(|v| {
            v["repository"]
                .as_str()
                .is_some_and(|repo| seen.insert(repo.to_owned()))
        })
        .collect()
}

fn recommend_cache_key(source: Source, engine: Engine, cap: &Capacity, hw: &Value) -> String {
    let ram_gb = hw["memoryBytes"].as_u64().unwrap_or(0) / GIB;
    let disk_gb = cap.disk / GIB;
    format!(
        "v1|{}|{}|{}|{}g|{}g",
        source.name(),
        engine.name(),
        cap.device,
        ram_gb,
        disk_gb / 10 * 10
    )
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedRecommendation {
    candidates: Vec<Value>,
    recommendation_source: String,
}

pub(crate) fn recommend(engine: &str, source: &str, query: &str) -> Result<Value, String> {
    let query = query.trim();
    let requested_source = Source::parse_or_default(source)?;
    let default_source = Source::default_for_environment();
    let direct = parse_direct_input(query);
    if direct.is_none()
        && (query.len() > 255
            || query.chars().any(char::is_control)
            || (query.contains('/') && !hub::valid_repository(query)))
    {
        return Err("Enter a model name, owner/repository, or Hugging Face / ModelScope URL".into());
    }
    let engine = Engine::for_model(engine)?;
    let hw = device::snapshot();
    let effective_source = direct
        .as_ref()
        .and_then(|d| d.source)
        .unwrap_or(requested_source);
    let source_auto_detected = direct.as_ref().is_some_and(|d| d.source.is_some());
    let origin = if direct.is_some() {
        "direct"
    } else if query.is_empty() {
        "hub"
    } else {
        "search"
    };
    let cap = match Capacity::for_engine(engine, &hw) {
        Ok(c) => c,
        Err(reason) => {
            return Ok(json!({
                "hardware": hw,
                "candidates": [],
                "recommendationSource": origin,
                "resolvedSource": effective_source.name(),
                "sourceAutoDetected": source_auto_detected,
                "defaultSource": default_source.name(),
                "reason": reason
            }));
        }
    };

    // Direct URL or owner/repo input: bypass keyword search and preview the repository directly.
    if let Some(target) = direct {
        let hub = Hub::new(effective_source)?;
        let preview = preview_with_file(
            &hub,
            engine,
            effective_source,
            &target.repository,
            target.filename.as_deref(),
            &cap,
        )?;
        return Ok(json!({
            "hardware": hw,
            "candidates": [preview.candidate],
            "recommendationSource": "direct",
            "resolvedSource": effective_source.name(),
            "sourceAutoDetected": source_auto_detected,
            "resolvedRepository": target.repository,
            "defaultSource": default_source.name(),
            "reason": Value::Null
        }));
    }

    // Fast path: check 30-min memory+disk cache for empty-query hardware recommendations.
    if query.is_empty() {
        let cache_key = recommend_cache_key(effective_source, engine, &cap, &hw);
        if let Some(cached) = cache::get::<CachedRecommendation>("recommend", &cache_key) {
            return Ok(json!({
                "hardware": hw,
                "candidates": cached.candidates,
                "recommendationSource": cached.recommendation_source,
                "resolvedSource": effective_source.name(),
                "sourceAutoDetected": false,
                "defaultSource": default_source.name(),
                "cached": true,
                "reason": Value::Null
            }));
        }
    }

    let deadline = Instant::now() + RECOMMEND_DEADLINE;
    let hub = Hub::with_deadline(effective_source, deadline)?;
    let mut candidates = Vec::new();
    let mut origin = origin;
    if query.is_empty() {
        if let Ok((queries, source_tag)) = modelfit::queries(engine, effective_source, &hw) {
            candidates = catalog_candidates_with_deadline(
                &queries,
                engine,
                Some(deadline),
                |query, mode| {
                    collect(&hub, engine, effective_source, query, &cap, mode)
                        .ok()?
                        .into_iter()
                        .next()
                },
            );
            if !candidates.is_empty() {
                origin = source_tag;
            }
        }
    }
    if candidates.is_empty() && !hub.deadline_exceeded() {
        candidates = collect(&hub, engine, effective_source, query, &cap, Lookup::Search)?;
    } else if query.is_empty()
        && !hub.deadline_exceeded()
        && candidates.iter().filter(|c| target_met(c)).count() < 3
    {
        // A memory tier may supply fewer than three eligible long-window models.
        // Fill any remaining long-window recommendations from the selected Hub.
        if let Ok(alternatives) = collect(&hub, engine, effective_source, "", &cap, Lookup::Search)
        {
            for mut item in alternatives.into_iter().filter(target_met) {
                if candidates
                    .iter()
                    .all(|saved| saved["repository"] != item["repository"])
                {
                    item["recommendationSource"] = json!("hub");
                    candidates.push(item);
                }
            }
        }
    }
    rank_candidates(&mut candidates);
    candidates.truncate(if query.is_empty() { 6 } else { 4 });
    if candidates
        .iter()
        .any(|item| item["recommendationSource"] == "hub")
    {
        origin = if candidates
            .iter()
            .all(|item| item["recommendationSource"] == "hub")
        {
            "hub"
        } else {
            "mixed"
        };
    }
    if query.is_empty() && !candidates.is_empty() {
        let cache_key = recommend_cache_key(effective_source, engine, &cap, &hw);
        cache::set(
            "recommend",
            &cache_key,
            &CachedRecommendation {
                candidates: candidates.clone(),
                recommendation_source: origin.to_owned(),
            },
            cache::RECOMMEND_TTL,
        );
    }
    let reason = candidates.is_empty().then_some(
        "No compatible snapshot fits the available memory and disk. Try another model name, source or engine.",
    );
    Ok(json!({
        "hardware": hw,
        "candidates": candidates,
        "recommendationSource": origin,
        "resolvedSource": effective_source.name(),
        "sourceAutoDetected": false,
        "defaultSource": default_source.name(),
        "reason": reason
    }))
}

fn load_cached_preview(
    source: Source,
    engine: Engine,
    repo: &str,
    plan: &Value,
) -> Option<CachedPreview> {
    let rev = plan["revision"].as_str();
    let var = plan["variant"].as_str();
    let specific_key = preview_cache_key(source, engine, repo, rev, var);
    if let Some(cached) = cache::get::<CachedPreview>("preview", &specific_key) {
        if check_confirmation(&cached.plan, plan).is_ok() {
            return Some(cached);
        }
    }
    let generic_key = preview_cache_key(source, engine, repo, None, None);
    if let Some(cached) = cache::get::<CachedPreview>("preview", &generic_key) {
        if check_confirmation(&cached.plan, plan).is_ok() {
            return Some(cached);
        }
    }
    None
}

pub(crate) fn install_plan(
    engine: &str,
    source: &str,
    plan: &Value,
    progress: impl FnMut(u64, u64) -> bool,
) -> Result<Value, String> {
    let engine = Engine::for_model(engine)?;
    let source = Source::parse_or_default(source)?;
    if plan["schemaVersion"] != 1
        || plan["engine"] != engine.name()
        || plan["source"] != source.name()
    {
        return Err(CHANGED.into());
    }
    let repo = plan["repository"]
        .as_str()
        .filter(|r| hub::valid_repository(r))
        .ok_or(CHANGED)?;
    let hub = Hub::new(source)?;
    if let Some(cached) = load_cached_preview(source, engine, repo, plan) {
        return activate(
            engine,
            source,
            repo,
            &hub,
            cached.manifest,
            cached.files,
            Some(&cached.plan),
            progress,
        );
    }
    let cap = Capacity::for_engine(engine, &device::snapshot())?;
    let requested_file = (engine == Engine::Llama)
        .then(|| plan["variant"].as_str())
        .flatten()
        .filter(|f| f.to_ascii_lowercase().ends_with(".gguf"));
    let preview = preview_with_file(&hub, engine, source, repo, requested_file, &cap)
        .map_err(|e| format!("{CHANGED} {e}"))?;
    check_confirmation(&preview.plan, plan)?;
    activate(
        engine,
        source,
        repo,
        &hub,
        preview.manifest,
        preview.files,
        Some(&preview.plan),
        progress,
    )
}

fn check_confirmation(current: &Value, confirmed: &Value) -> Result<(), String> {
    // Validate core security and integrity fields rather than brittle full-object equality
    // so harmless metadata differences (e.g. timestamps or additive fields) do not reject installs.
    if current["repository"] != confirmed["repository"]
        || current["revision"] != confirmed["revision"]
        || current["contextTokens"] != confirmed["contextTokens"]
    {
        return Err(CHANGED.into());
    }
    for key in ["engine", "source", "runtimeBackend"] {
        if current.get(key).is_some() || confirmed.get(key).is_some() {
            if current.get(key) != confirmed.get(key) {
                return Err(CHANGED.into());
            }
        }
    }
    let current_files = current["files"].as_array().ok_or(CHANGED)?;
    let confirmed_files = confirmed["files"].as_array().ok_or(CHANGED)?;
    if current_files.len() != confirmed_files.len() || current_files.is_empty() {
        return Err(CHANGED.into());
    }
    for (cur, conf) in current_files.iter().zip(confirmed_files.iter()) {
        if cur["path"] != conf["path"]
            || cur["bytes"] != conf["bytes"]
            || cur["revision"] != conf["revision"]
        {
            return Err(CHANGED.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimates_use_shared_memory_once_and_leave_room_for_kv_cache() {
        let shared = json!({"memoryBytes":12*GIB,"gpuMemoryBytes":12*GIB,"unifiedMemory":true,"freeDiskBytes":32*GIB});
        let cap = Capacity::for_engine(Engine::Llama, &shared).unwrap();
        assert_eq!(cap.memory, 8 * GIB);
        let config = json!({"max_position_embeddings":3072,"num_hidden_layers":32,"num_key_value_heads":8,"head_dim":128});
        let (ctx, est) = estimate(&cap, 4 * GIB, 0, &config, None).unwrap();
        assert_eq!(ctx, 3072);
        assert!(est > 4 * GIB && est <= cap.memory);
        assert!(estimate(&cap, 9 * GIB, 0, &config, None).is_err());
        let small_disk = Capacity {
            disk: 2 * GIB,
            memory: cap.memory,
            device: cap.device,
            target_context: cap.target_context,
        };
        assert!(estimate(&small_disk, 4 * GIB, 0, &config, None).is_err());
        assert!(estimate(&small_disk, 4 * GIB, 3 * GIB, &config, None).is_ok());
    }
    #[test]
    fn long_window_requires_native_length_and_complete_cache_dimensions() {
        let cfg = json!({"max_position_embeddings":262144,"head_dim":64,"num_hidden_layers":16,"num_key_value_heads":2});
        let device = Capacity {
            memory: 12 * GIB,
            disk: 30 * GIB,
            device: "unified",
            target_context: TARGET_CONTEXT,
        };
        let arch = Architecture::detect(&cfg, None);
        assert_eq!(arch.kv, Some(8192));
        let (window, bytes) = estimate_with_arch(&device, 2 * GIB, 0, &arch).unwrap();
        assert_eq!(window, TARGET_CONTEXT);
        assert!(bytes <= device.memory);
        assert_eq!(arch.reason(window), "target_met");
        let capped = Architecture::detect(&cfg, Some(64_000));
        assert_eq!(
            estimate_with_arch(&device, 2 * GIB, 0, &capped).unwrap().0,
            64_000
        );
        let missing = Architecture::detect(&json!({"max_position_embeddings":262144}), None);
        let short = estimate_with_arch(&device, 2 * GIB, 0, &missing).unwrap().0;
        assert!(short <= 8192);
        assert_eq!(missing.reason(short), "metadata_unknown");
        let limited = Capacity {
            memory: 3 * GIB,
            ..device
        };
        let shorter = estimate_with_arch(&limited, 2 * GIB, 0, &arch).unwrap().0;
        assert!(shorter < TARGET_CONTEXT);
        assert_eq!(arch.reason(shorter), "memory_budget");
    }
    #[test]
    fn hybrid_attention_budgets_full_kv_and_recurrent_state() {
        let cfg = json!({"text_config":{"model_type":"qwen3_5_text","max_position_embeddings":262144,
         "layer_types":["linear_attention","full_attention"],"num_hidden_layers":2,"num_key_value_heads":2,
         "head_dim":256,"linear_num_key_heads":16,"linear_num_value_heads":32,"linear_key_head_dim":128,
         "linear_value_head_dim":128,"linear_conv_kernel_dim":4}});
        let arch = Architecture::detect(&cfg, None);
        assert_eq!(arch.kv, Some(2048));
        assert_eq!(
            arch.static_bytes(),
            512 * MIB + 4 * (32 * 128 * 128 + 8192 * 4)
        );
        assert!(weights_quantized(
            &json!({"quantization":{"bits":4,"group_size":64}})
        ));
        assert!(!weights_quantized(&json!({"dtype":"bfloat16"})));
    }
    #[test]
    fn unknown_gpu_is_not_treated_as_dedicated_vram() {
        let hw = json!({"memoryBytes":16*GIB,"gpu":"AMD","freeDiskBytes":32*GIB});
        assert!(Capacity::for_engine(Engine::Vllm, &hw).is_err());
        assert!(Capacity::for_engine(Engine::Mlx, &hw).is_err());
        assert_eq!(
            Capacity::for_engine(Engine::Llama, &hw).unwrap().device,
            "cpu"
        );
    }
    #[test]
    fn every_confirmed_file_revision_size_and_context_must_still_match() {
        let plan = json!({"repository":"a/model","revision":"aa","files":[{"path":"x.gguf","bytes":14,"revision":"bb"}],"contextTokens":2048});
        assert!(check_confirmation(&plan, &plan).is_ok());
        for (p, v) in [
            ("/repository", json!("b/model")),
            ("/revision", json!("ab")),
            ("/files/0/bytes", json!(15)),
            ("/files/0/revision", json!("bc")),
            ("/files/0/path", json!("y.gguf")),
            ("/contextTokens", json!(4096)),
        ] {
            let mut changed = plan.clone();
            *changed.pointer_mut(p).unwrap() = v;
            assert!(check_confirmation(&plan, &changed).is_err());
        }
    }
    #[test]
    fn new_quantization_keeps_the_existing_snapshot_and_its_partial_download() {
        let root = tempfile::tempdir().unwrap();
        let snap = root.path().join("snapshots/r");
        fs::create_dir_all(&snap).unwrap();
        storage::write_json(
            &snap.join("snapshot.json"),
            &json!({"artifacts":[{"path":"a.gguf"}]}),
        )
        .unwrap();
        let files = ["a.gguf", "b.gguf"].map(|path| FileSpec {
            path: path.into(),
            bytes: 4,
            revision: "r".into(),
            sha256: None,
        });
        let manifest = Manifest {
            revision: "r".into(),
            files: files.to_vec(),
            context: None,
            tags: Value::Null,
        };
        assert_eq!(
            snapshot_names(root.path(), Engine::Llama, &manifest, &files[..1])
                .unwrap()
                .0,
            "r"
        );
        assert_eq!(
            snapshot_names(root.path(), Engine::Llama, &manifest, &files[1..]).unwrap(),
            ("r-gguf-1".into(), ".partial-r-gguf-1".into())
        );
        let old = root.path().join(".partial-r");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("b.gguf.part"), [0, 1]).unwrap();
        assert_eq!(
            snapshot_names(root.path(), Engine::Llama, &manifest, &files[1..])
                .unwrap()
                .1,
            ".partial-r"
        );
    }
    #[test]
    fn rank_accounts_for_context_before_quantization_publisher() {
        let candidate = |repo: &str, ctx: u64, quantized: bool| {
            json!({
                "repository":repo, "contextTokens":ctx, "quantized":quantized,
                "contextLimitReason": if ctx == TARGET_CONTEXT { "target_met" } else { "memory_budget" }
            })
        };
        let mut options = vec![
            candidate("unsloth/small-window", 8192, true),
            candidate("mirror/unsloth-name", TARGET_CONTEXT, true),
            candidate("unsloth/full-precision", TARGET_CONTEXT, false),
            candidate("UNSLOTH/official", TARGET_CONTEXT, true),
        ];
        rank_candidates(&mut options);
        assert_eq!(options[0]["repository"], "UNSLOTH/official");
        assert_eq!(options[1]["repository"], "mirror/unsloth-name");
        assert_eq!(
            options.last().unwrap()["repository"],
            "unsloth/small-window"
        );
    }

    #[test]
    fn catalog_checks_other_publishers_only_when_preferred_plans_are_insufficient() {
        let families: Vec<_> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|q| q.to_string())
            .collect();
        let plan = |repo: &str| json!({"repository":repo,"contextTokens":TARGET_CONTEXT,"contextLimitReason":"target_met"});
        let calls = std::sync::Mutex::new(Vec::new());
        let best = catalog_candidates(&families, Engine::Llama, |name, mode| {
            calls.lock().unwrap().push((name.to_owned(), mode));
            let repo = if matches!(name, "a" | "b") { "a" } else { name };
            Some(plan(&format!("unsloth/{repo}")))
        });
        let first_calls = calls.into_inner().unwrap();
        assert_eq!(first_calls.len(), 4);
        assert!(first_calls
            .iter()
            .all(|(name, mode)| *mode == Lookup::PreferredFamily
                && !matches!(name.as_str(), "e" | "f")));
        assert_eq!(best.len(), 3);
        assert_eq!(best[0]["repository"], "unsloth/a");

        let broad_calls = std::sync::Mutex::new(Vec::new());
        let fallback = catalog_candidates(&families, Engine::Llama, |name, mode| {
            if mode == Lookup::Family {
                broad_calls.lock().unwrap().push(name.to_owned());
            }
            match (mode, name) {
                (Lookup::PreferredFamily, "e" | "f") => Some(plan(name)),
                (Lookup::Family, "a") => Some(plan("other/a")),
                _ => None,
            }
        });
        let mut broad_calls = broad_calls.into_inner().unwrap();
        broad_calls.sort();
        assert_eq!(broad_calls, ["a", "b"]);
        assert_eq!(
            fallback
                .iter()
                .map(|v| v["repository"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["other/a", "e", "f"]
        );
    }

    #[test]
    fn explicitly_chat_capable_tokenizers_only() {
        assert!(has_chat_template(
            &json!([{"name":"default","template":"{{ messages }}"}])
        ));
        assert!(has_chat_template(&json!({"default":"{{ messages }}"})));
        assert!(!has_chat_template(&json!({"default":" "})));
        assert!(!has_chat_template(&json!([{"name":"default"}])));
        assert!(!has_chat_template(&Value::Null));
    }
    #[test]
    fn matching_catalog_model_preserves_size_family_and_instruction_tuning() {
        assert!(family_matches(
            "publisher/Qwen2.5-3B-Instruct-GGUF",
            "Qwen2.5-3B-Instruct"
        ));
        assert!(!family_matches(
            "publisher/Qwen2.5-13B-Instruct-GGUF",
            "Qwen2.5-3B-Instruct"
        ));
        assert!(!family_matches(
            "publisher/Qwen2.5-3B-Instruct-lora",
            "Qwen2.5-3B-Instruct"
        ));
    }

    #[test]
    fn adaptive_context_target_scales_with_device_ram() {
        assert_eq!(adaptive_target_context(4 * GIB), 8_192);
        assert_eq!(adaptive_target_context(8 * GIB), 8_192);
        assert_eq!(adaptive_target_context(16 * GIB), 32_768);
        assert_eq!(adaptive_target_context(24 * GIB), 65_536);
        assert_eq!(adaptive_target_context(32 * GIB), 65_536);
        assert_eq!(adaptive_target_context(64 * GIB), 131_072);

        let hw_16g = json!({"memoryBytes":16*GIB,"unifiedMemory":true,"freeDiskBytes":64*GIB});
        let cap = Capacity::for_engine(Engine::Llama, &hw_16g).unwrap();
        assert_eq!(cap.target_context, 32_768);

        let met = json!({
            "contextTokens": 32_768,
            "targetContextTokens": 32_768,
            "contextLimitReason": "target_met"
        });
        assert!(target_met(&met));
    }

    #[test]
    fn direct_input_parses_hf_modelscope_urls_and_owner_repo() {
        assert_eq!(
            parse_direct_input("https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF"),
            Some(DirectTarget {
                source: Some(Source::HuggingFace),
                repository: "Qwen/Qwen2.5-7B-Instruct-GGUF".into(),
                filename: None,
            })
        );
        assert_eq!(
            parse_direct_input(
                "https://hf-mirror.com/unsloth/Qwen3-8B-GGUF/blob/main/Qwen3-8B-Q4_K_M.gguf?download=true"
            ),
            Some(DirectTarget {
                source: Some(Source::HuggingFace),
                repository: "unsloth/Qwen3-8B-GGUF".into(),
                filename: Some("Qwen3-8B-Q4_K_M.gguf".into()),
            })
        );
        assert_eq!(
            parse_direct_input("https://modelscope.cn/models/Qwen/Qwen2.5-7B-Instruct-GGUF/files"),
            Some(DirectTarget {
                source: Some(Source::ModelScope),
                repository: "Qwen/Qwen2.5-7B-Instruct-GGUF".into(),
                filename: None,
            })
        );
        assert_eq!(
            parse_direct_input(
                "https://www.modelscope.cn/models/unsloth/Qwen3-8B-GGUF/file/view/master/Qwen3-8B-Q4_K_M.gguf"
            ),
            Some(DirectTarget {
                source: Some(Source::ModelScope),
                repository: "unsloth/Qwen3-8B-GGUF".into(),
                filename: Some("Qwen3-8B-Q4_K_M.gguf".into()),
            })
        );
        assert_eq!(
            parse_direct_input("Qwen/Qwen2.5-7B-Instruct-GGUF"),
            Some(DirectTarget {
                source: None,
                repository: "Qwen/Qwen2.5-7B-Instruct-GGUF".into(),
                filename: None,
            })
        );
        // Plain search keywords are not direct targets
        assert_eq!(parse_direct_input("qwen 7b"), None);
        // Non-model HF URLs or arbitrary hosts are rejected
        assert_eq!(
            parse_direct_input("https://huggingface.co/datasets/foo/bar"),
            None
        );
        assert_eq!(parse_direct_input("https://example.com/a/b"), None);
    }

    #[test]
    fn confirmation_reuses_cached_preview_and_ignores_harmless_metadata() {
        let plan = json!({
            "schemaVersion": 1,
            "engine": "llama.cpp",
            "source": "modelscope",
            "repository": "unsloth/Qwen3-8B-GGUF",
            "revision": "a".repeat(40),
            "variant": "Qwen3-8B-Q4_K_M.gguf",
            "files": [{"path": "Qwen3-8B-Q4_K_M.gguf", "bytes": 4096, "revision": "a".repeat(40)}],
            "contextTokens": 32768,
            "targetContextTokens": 32768,
            "estimatedMemoryBytes": 6000
        });
        let mut harmless = plan.clone();
        harmless["generatedAt"] = json!("2026-11-22T06:00:00Z");
        harmless["estimatedMemoryBytes"] = json!(6123);
        harmless["cached"] = json!(true);
        assert!(check_confirmation(&plan, &harmless).is_ok());

        let cached = CachedPreview {
            manifest: Manifest {
                revision: "a".repeat(40),
                files: vec![FileSpec {
                    path: "Qwen3-8B-Q4_K_M.gguf".into(),
                    bytes: 4096,
                    revision: "a".repeat(40),
                    sha256: None,
                }],
                context: Some(32768),
                tags: Value::Null,
            },
            files: vec![FileSpec {
                path: "Qwen3-8B-Q4_K_M.gguf".into(),
                bytes: 4096,
                revision: "a".repeat(40),
                sha256: None,
            }],
            plan: plan.clone(),
        };
        let key = preview_cache_key(
            Source::ModelScope,
            Engine::Llama,
            "unsloth/Qwen3-8B-GGUF",
            Some(&"a".repeat(40)),
            Some("Qwen3-8B-Q4_K_M.gguf"),
        );
        cache::set("preview", &key, &cached, cache::PREVIEW_TTL);
        let reused = load_cached_preview(
            Source::ModelScope,
            Engine::Llama,
            "unsloth/Qwen3-8B-GGUF",
            &harmless,
        );
        assert!(reused.is_some());
        assert_eq!(reused.unwrap().files[0].path, "Qwen3-8B-Q4_K_M.gguf");
    }

    #[test]
    fn catalog_respects_deadline_and_returns_partial_results() {
        let families: Vec<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|q| q.to_string())
            .collect();
        let expired = Instant::now() - Duration::from_secs(1);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let res = catalog_candidates_with_deadline(
            &families,
            Engine::Llama,
            Some(expired),
            |_name, _mode| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                None
            },
        );
        assert!(res.is_empty());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
