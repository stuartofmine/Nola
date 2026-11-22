// SPDX-License-Identifier: Apache-2.0
use super::{cache, Engine, FileSpec, Manifest, Source};
use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
    Url,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

type MetadataSlots<T> = Mutex<HashMap<String, Arc<OnceLock<Result<T, String>>>>>;

fn shared_metadata<T: Clone>(
    cache: &MetadataSlots<T>,
    key: &str,
    load: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let slot = {
        let mut map = cache
            .lock()
            .map_err(|_| "Model metadata cache is unavailable")?;
        map.entry(key.to_owned()).or_default().clone()
    };
    // Release the maps lock before doing network I/O. Only readers of the same
    // resource wait for this lookup; unrelated models are resolved concurrently.
    slot.get_or_init(load).clone()
}

pub(super) struct Hub {
    client: Client,
    source: Source,
    deadline: Option<Instant>,
    // Each recommendation and each confirmation creates its own Hub.
    preferred_lists: MetadataSlots<Vec<String>>,
    manifests: MetadataSlots<Manifest>,
    configurations: MetadataSlots<Value>,
    quantization_bases: MetadataSlots<Option<String>>,
}

fn origin(source: Source) -> &'static str {
    match source {
        Source::HuggingFace => "https://huggingface.co",
        Source::ModelScope => "https://modelscope.cn",
    }
}
fn allowed_redirect(url: &Url, source: Source) -> bool {
    let hosts: &[&str] = match source {
        Source::HuggingFace => &["huggingface.co", "hf.co"],
        Source::ModelScope => &["modelscope.cn", "modelscope.net", "aliyuncs.com"],
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            hosts
                .iter()
                .any(|parent| host == *parent || host.ends_with(&format!(".{parent}")))
        })
}

fn metadata(response: Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Repository metadata request failed (HTTP {status})"
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Repository metadata is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid repository metadata".into())
}

pub(super) fn valid_repository(repo: &str) -> bool {
    let parts: Vec<_> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 96
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !part.contains("..")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
        })
}
fn immutable(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.bytes().all(|b| b.is_ascii_hexdigit())
}
impl Hub {
    pub(super) fn new(source: Source) -> Result<Self, String> {
        Self::build(source, None)
    }

    pub(super) fn with_deadline(source: Source, deadline: Instant) -> Result<Self, String> {
        Self::build(source, Some(deadline))
    }

    fn build(source: Source, deadline: Option<Instant>) -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // The blocking client wraps each Read with this timeout; keep weight
            // requests free of a separate total-transfer deadline.
            .timeout(Duration::from_secs(30))
            .user_agent("Nola/1.0")
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() < 7 && allowed_redirect(attempt.url(), source) {
                    attempt.follow()
                } else {
                    attempt.error("Invalid model download redirect")
                }
            }))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            source,
            deadline,
            preferred_lists: Mutex::default(),
            manifests: Mutex::default(),
            configurations: Mutex::default(),
            quantization_bases: Mutex::default(),
        })
    }

    pub(super) fn deadline_exceeded(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    fn url(&self, parts: &[&str]) -> Url {
        let mut url = Url::parse(origin(self.source)).expect("official model source");
        url.path_segments_mut().expect("http").extend(parts);
        url
    }

    fn get(&self, url: Url) -> Result<Value, String> {
        let timeout = if let Some(deadline) = self.deadline {
            let now = Instant::now();
            if now >= deadline {
                return Err("Recommendation deadline exceeded".into());
            }
            deadline.saturating_duration_since(now).min(Duration::from_secs(10))
        } else {
            Duration::from_secs(10)
        };
        let response = self
            .client
            .get(url)
            .timeout(timeout)
            .send()
            .map_err(|e| e.without_url().to_string())?;
        metadata(response)
    }

    pub(super) fn search(&self, engine: Engine, query: &str) -> Result<Value, String> {
        let results = self.find(engine, query, 4, false, |repo| {
            self.candidate(engine, repo, valid_repository(query))
        })?;
        if valid_repository(query) && results.is_empty() {
            return Err(format!(
                "No complete {} model found in {query}",
                engine.name()
            ));
        }
        Ok(json!({"candidates":results}))
    }

    fn repository_ids(
        &self,
        engine: Engine,
        query: &str,
        preferred_publisher: bool,
    ) -> Result<Vec<String>, String> {
        let term = if query.is_empty() { "Instruct" } else { query };
        let mut candidates = Vec::new();
        match self.source {
            Source::HuggingFace => {
                let mut url = self.url(&["api", "models"]);
                let filter = match engine {
                    Engine::Llama => "gguf",
                    Engine::Mlx => "mlx",
                    Engine::Vllm => "safetensors",
                };
                url.query_pairs_mut()
                    .append_pair("search", term)
                    .append_pair("filter", filter)
                    .append_pair("sort", "downloads")
                    .append_pair("limit", "32");
                if query.is_empty() {
                    url.query_pairs_mut()
                        .append_pair("pipeline_tag", "text-generation");
                }
                if preferred_publisher {
                    url.query_pairs_mut().append_pair("author", "unsloth");
                }
                let values = self.get(url)?;
                let items = values
                    .as_array()
                    .ok_or("Invalid Hugging Face search response")?;
                for item in items {
                    let repo = item["id"].as_str().unwrap_or_default();
                    if valid_repository(repo) && engine.matches_metadata(repo, &item["tags"]) {
                        candidates.push(repo.to_owned());
                    }
                }
            }
            Source::ModelScope => {
                let mut url = self.url(&["openapi", "v1", "models"]);
                let query = match engine {
                    Engine::Llama if !term.to_ascii_lowercase().contains("gguf") => {
                        format!("{term} GGUF")
                    }
                    Engine::Mlx if !term.to_ascii_lowercase().contains("mlx") => {
                        format!("{term} MLX")
                    }
                    _ => term.to_owned(),
                };
                // Ask for format-specific results before searching the base name.
                url.query_pairs_mut()
                    .append_pair("search", &query)
                    .append_pair("page_size", "50");
                if preferred_publisher {
                    url.query_pairs_mut().append_pair("owner", "unsloth");
                }
                let mut values = self.get(url.clone())?;
                if values["data"]["models"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                    && query != term
                {
                    url.query_pairs_mut()
                        .clear()
                        .append_pair("search", term)
                        .append_pair("page_size", "50");
                    if preferred_publisher {
                        url.query_pairs_mut().append_pair("owner", "unsloth");
                    }
                    values = self.get(url)?;
                }
                for item in values["data"]["models"]
                    .as_array()
                    .ok_or("Invalid ModelScope search response")?
                {
                    let repo = item["id"].as_str().unwrap_or_default();
                    if valid_repository(repo) && engine.matches_metadata(repo, &item["tags"]) {
                        candidates.push(repo.to_owned());
                    }
                }
            }
        }
        if preferred_publisher {
            candidates.retain(|repo| super::planning::unsloth_repository(repo));
        }
        Ok(candidates)
    }

    pub(super) fn preferred_repositories(
        &self,
        engine: Engine,
        query: &str,
    ) -> Result<Vec<String>, String> {
        let key = format!("{}/{}", engine.name(), query);
        shared_metadata(&self.preferred_lists, &key, || {
            self.repository_ids(engine, query, true)
        })
    }

    pub(super) fn find(
        &self,
        engine: Engine,
        query: &str,
        limit: usize,
        recommend: bool,
        inspect: impl Fn(&str) -> Result<Option<Value>, String> + Sync,
    ) -> Result<Vec<Value>, String> {
        if valid_repository(query) {
            return Ok(inspect(query)?.into_iter().collect());
        }
        let metal_mlx = engine == Engine::Vllm
            && super::VllmBackend::current().ok() == Some(super::VllmBackend::MetalMlx);
        let candidates = if recommend || metal_mlx {
            // Hub popularity lists may omit machine-specific conversions. Mix a
            // small MLX sample into the general list so BF16 and AWQ remain eligible.
            let (preferred, mlx, general) = std::thread::scope(|scope| {
                let preferred =
                    recommend.then(|| scope.spawn(|| self.preferred_repositories(engine, query)));
                let mlx = metal_mlx
                    .then(|| scope.spawn(|| self.repository_ids(Engine::Mlx, query, false)));
                let general = self.repository_ids(engine, query, false);
                (
                    preferred.and_then(|w| w.join().ok()),
                    mlx.and_then(|w| w.join().ok()),
                    general,
                )
            });
            let mut ids = preferred.and_then(Result::ok).unwrap_or_default();
            ids.truncate(if metal_mlx { 2 } else { 6 });
            ids.extend(mlx.and_then(Result::ok).into_iter().flatten().take(4));
            match general {
                Ok(other) => ids.extend(other),
                Err(e) if ids.is_empty() => return Err(e),
                Err(_) => {}
            }
            let mut seen = std::collections::HashSet::new();
            ids.retain(|r| seen.insert(r.to_ascii_lowercase()));
            ids
        } else {
            self.repository_ids(engine, query, false)?
        };
        // Tags are advisory. Some popular "safetensors" results contain only ONNX or pickle.
        // Inspect manifests concurrently but preserve the hub's ranking when presenting matches.
        let mut matches = Vec::new();
        let mut first_error = None;
        let mut inspected = 0;
        let max_chunks = if recommend { 2 } else { 4 };
        for group in candidates.chunks(4).take(max_chunks) {
            if self.deadline_exceeded() {
                break;
            }
            let results = std::thread::scope(|scope| {
                let workers: Vec<_> = group
                    .iter()
                    .map(|repo| scope.spawn(|| inspect(repo)))
                    .collect();
                workers
                    .into_iter()
                    .map(|w| {
                        w.join()
                            .unwrap_or_else(|_| Err("Model lookup was interrupted".into()))
                    })
                    .collect::<Vec<_>>()
            });
            for result in results {
                match result {
                    Ok(candidate) => {
                        inspected += 1;
                        matches.extend(candidate);
                    }
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if recommend {
                super::planning::rank_candidates(&mut matches);
            }
            if matches.len() >= limit
                && (!recommend || super::planning::target_met(&matches[limit - 1]))
            {
                break;
            }
        }
        if inspected == 0 {
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        matches.truncate(limit);
        Ok(matches)
    }

    fn candidate(&self, engine: Engine, repo: &str, exact: bool) -> Result<Option<Value>, String> {
        let manifest = self.manifest(repo)?;
        let files = match super::resolved_files(self, engine, repo, &manifest) {
            Ok(files) => files,
            Err(reason) if exact => return Err(reason),
            Err(_) => return Ok(None),
        };
        let bytes = files
            .iter()
            .try_fold(0u64, |n, f| n.checked_add(f.bytes))
            .ok_or("Invalid model size")?;
        let variant = if engine == Engine::Llama {
            Some(files[0].path.clone())
        } else if engine == Engine::Vllm
            && super::VllmBackend::current()? != super::VllmBackend::Standard
        {
            let file = files
                .iter()
                .find(|f| f.path == "config.json")
                .ok_or("Missing model configuration")?;
            let cfg = self.configuration(repo, file)?;
            super::VllmBackend::current()?
                .checkpoint(&cfg)?
                .or_else(|| {
                    cfg.get("dtype")
                        .or(cfg.get("torch_dtype"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
        } else {
            None
        };
        Ok(Some(
            json!({"repository":repo,"revision":manifest.revision,"downloadBytes":bytes,"variant":variant}),
        ))
    }

    pub(super) fn manifest(&self, repo: &str) -> Result<Manifest, String> {
        shared_metadata(&self.manifests, repo, || {
            let cache_key = format!("{}|{}", self.source.name(), repo);
            if let Some(cached) = cache::get::<Manifest>("hub_manifest", &cache_key) {
                return Ok(cached);
            }
            let fetched = self.fetch_manifest(repo)?;
            cache::set(
                "hub_manifest",
                &cache_key,
                &fetched,
                cache::HUB_METADATA_TTL,
            );
            Ok(fetched)
        })
    }

    pub(super) fn configuration(&self, repo: &str, file: &FileSpec) -> Result<Value, String> {
        if file.bytes > 4 * 1024 * 1024 {
            return Err("Repository model configuration is too large".into());
        }
        let key = format!("{repo}@{}/{}", file.revision, file.path);
        shared_metadata(&self.configurations, &key, || {
            let cache_key = format!("{}|{key}", self.source.name());
            if let Some(cached) = cache::get::<Value>("hub_config", &cache_key) {
                return Ok(cached);
            }
            let mut data = Vec::new();
            self.artifact(repo, file, 0)?
                .take(file.bytes + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            if data.len() as u64 != file.bytes {
                return Err("Incomplete model configuration".into());
            }
            let parsed: Value =
                serde_json::from_slice(&data).map_err(|_| "Invalid model configuration")?;
            cache::set("hub_config", &cache_key, &parsed, cache::HUB_METADATA_TTL);
            Ok(parsed)
        })
    }

    fn fetch_manifest(&self, repo: &str) -> Result<Manifest, String> {
        if !valid_repository(repo) {
            return Err("Choose a model repository with an owner and name".into());
        }
        let (owner, name) = repo.split_once('/').unwrap();
        let (revision, rows, ctx, tags) = match self.source {
            Source::HuggingFace => {
                let mut url = self.url(&["api", "models", owner, name]);
                url.query_pairs_mut().append_pair("blobs", "true");
                let doc = self.get(url)?;
                (
                    doc["sha"]
                        .as_str()
                        .filter(|s| immutable(s))
                        .ok_or("Missing immutable Hugging Face revision")?
                        .to_owned(),
                    doc["siblings"]
                        .as_array()
                        .ok_or("Missing model files")?
                        .clone(),
                    doc["gguf"]["context_length"].as_u64(),
                    doc["tags"].clone(),
                )
            }
            Source::ModelScope => {
                let mut url = self.url(&["api", "v1", "models", owner, name, "repo", "files"]);
                url.query_pairs_mut().append_pair("Recursive", "true");
                let doc = self.get(url)?;
                let data = &doc["Data"];
                let commit = &data["LatestCommitter"];
                let rev = commit["Id"]
                    .as_str()
                    .filter(|s| immutable(s))
                    .or_else(|| {
                        commit["ShortId"].as_str().filter(|s| {
                            (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    })
                    .ok_or("Missing ModelScope revision")?;
                (
                    rev.to_owned(),
                    data["Files"]
                        .as_array()
                        .ok_or("Missing ModelScope files")?
                        .clone(),
                    None,
                    Value::Null,
                )
            }
        };
        let mut files = Vec::new();
        for row in &rows {
            if self.source == Source::ModelScope && row["Type"] != "blob" {
                continue;
            }
            let (path, size, file_revision, sha) = match self.source {
                Source::HuggingFace => (
                    row["rfilename"].as_str(),
                    row["size"].as_u64(),
                    Some(revision.as_str()),
                    row["lfs"]["sha256"].as_str(),
                ),
                Source::ModelScope => (
                    row["Path"].as_str(),
                    row["Size"].as_u64(),
                    row["Revision"].as_str(),
                    row["Sha256"].as_str(),
                ),
            };
            let path = path.ok_or("Missing artifact path")?;
            let spec = FileSpec {
                path: path.into(),
                bytes: size.ok_or("Missing artifact size")?,
                revision: file_revision
                    .filter(|s| immutable(s))
                    .ok_or("Missing artifact revision")?
                    .into(),
                sha256: sha
                    .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_ascii_lowercase),
            };
            files.push(spec);
        }
        Ok(Manifest {
            revision,
            files,
            context: ctx,
            tags,
        })
    }

    /// Only follow one declared quantization base on this same Hub. Names inferred
    /// from an uploader or filename could resolve to a different architecture.
    pub(super) fn quantization_base(
        &self,
        repo: &str,
        manifest: &Manifest,
    ) -> Result<Option<String>, String> {
        shared_metadata(&self.quantization_bases, repo, || {
            let cache_key = format!("{}|{}|{}", self.source.name(), repo, manifest.revision);
            if let Some(cached) = cache::get::<Option<String>>("hub_qbase", &cache_key) {
                return Ok(cached);
            }
            let fetched = self.fetch_quantization_base(repo, manifest)?;
            cache::set("hub_qbase", &cache_key, &fetched, cache::HUB_METADATA_TTL);
            Ok(fetched)
        })
    }

    fn fetch_quantization_base(
        &self,
        repo: &str,
        manifest: &Manifest,
    ) -> Result<Option<String>, String> {
        let bases: Vec<String> = match self.source {
            Source::HuggingFace => manifest
                .tags
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|tag| tag.strip_prefix("base_model:quantized:"))
                .map(str::to_owned)
                .collect(),
            Source::ModelScope => {
                let (owner, name) = repo.split_once('/').ok_or("Invalid repository")?;
                let data = self.get(self.url(&["api", "v1", "models", owner, name]))?;
                let info = &data["Data"];
                if !info["BaseModelRelation"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("quantized"))
                {
                    return Ok(None);
                }
                info["BaseModel"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }
        };
        let mut bases = bases
            .into_iter()
            .filter(|r| valid_repository(r) && r != repo)
            .collect::<Vec<_>>();
        bases.sort();
        bases.dedup();
        Ok(if bases.len() == 1 { bases.pop() } else { None })
    }

    pub(super) fn artifact(
        &self,
        repo: &str,
        file: &FileSpec,
        offset: u64,
    ) -> Result<Response, String> {
        let (owner, name) = repo.split_once('/').ok_or("Invalid repository")?;
        let url = match self.source {
            Source::HuggingFace => {
                let mut url = self.url(&[owner, name, "resolve", &file.revision]);
                url.path_segments_mut()
                    .unwrap()
                    .extend(file.path.split('/'));
                url
            }
            Source::ModelScope => {
                let mut url = self.url(&["api", "v1", "models", owner, name, "repo"]);
                url.query_pairs_mut()
                    .append_pair("Revision", &file.revision)
                    .append_pair("FilePath", &file.path);
                url
            }
        };
        let mut req = self.client.get(url);
        if offset > 0 {
            req = req.header("Range", format!("bytes={offset}-"));
        }
        let response = req.send().map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "Download {} failed (HTTP {})",
                file.path,
                response.status()
            ));
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_conversions_share_their_base_config_only_within_one_lookup() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let first = MetadataSlots::<usize>::default();
        let load = || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(10));
            Ok(42)
        };
        std::thread::scope(|scope| {
            let workers = (0..8)
                .map(|_| scope.spawn(|| shared_metadata(&first, "base@rev/config.json", load)))
                .collect::<Vec<_>>();
            for w in workers {
                assert_eq!(w.join().unwrap().unwrap(), 42)
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let confirmation = MetadataSlots::default();
        assert_eq!(
            shared_metadata(&confirmation, "base@rev/config.json", load).unwrap(),
            42
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn hub_rejects_arbitrary_repositories_and_redirects() {
        for repo in [
            "../foo",
            "qwen/../file",
            "https://evil/x",
            "repo/x?x",
            "owner//repo",
        ] {
            assert!(!valid_repository(repo));
        }
        for url in [
            "http://huggingface.co/x",
            "https://hf.co.evil.test/x",
            "https://key@huggingface.co/x",
            "https://127.0.0.1/foo",
        ] {
            assert!(!allowed_redirect(
                &Url::parse(url).unwrap(),
                Source::HuggingFace
            ));
        }
        assert!(allowed_redirect(
            &Url::parse("https://cas-bridge.xethub.hf.co/x").unwrap(),
            Source::HuggingFace
        ));
    }
}
