// SPDX-License-Identifier: Apache-2.0
//! Install plain SKILL.md packages from public GitHub repositories.
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use url::Url;

const API: &str = "https://api.github.com";
const RAW: &str = "https://raw.githubusercontent.com";
pub const RESOURCE_TREE: &str = ".nola-package";
const PACKAGE_REFERENCE: &str = "nola://github-skills/";
const GITHUB_PREFIX: &str = "https://github.com/";
const PACKAGE_LIMIT: u64 = 32 * 1024 * 1024;

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent("Nola-Skill-Installer/1")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())
}
fn read_url(client: &Client, url: Url, limit: u64) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().map_err(|e| e.to_string())?;
    read_response(response, limit)
}

fn read_response(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>, String> {
    if rate_limited(&response) {
        return Err("GitHub API request quota exhausted. Retry after the quota resets, or use a public repository link to inspect its available Skills.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned {} for {}",
            response.status(),
            response.url()
        ));
    }
    let mut data = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err("GitHub response exceeds size limit".into());
    }
    Ok(data)
}
fn endpoint(host: &str, segments: &[&str]) -> Url {
    let mut url = Url::parse(host).expect("fixed GitHub host");
    url.path_segments_mut().unwrap().extend(segments);
    url
}
fn valid_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part.len() <= 240
        && !part.ends_with([' ', '.'])
        && !part
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        && !matches!(
            part.to_ascii_lowercase().as_str(),
            ".git" | ".package" | RESOURCE_TREE | "nola_manifest.json" | ".install-lock"
        )
        && !reserved_device_name(part)
}

fn reserved_device_name(part: &str) -> bool {
    let upper = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(
            upper.as_bytes(),
            [b'C', b'O', b'M', b'1'..=b'9'] | [b'L', b'P', b'T', b'1'..=b'9']
        )
}
fn slug(s: &str) -> bool {
    valid_component(s)
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
}
#[derive(Debug)]
struct Source {
    owner: String,
    repo: String,
    revision: String,
    path: String,
}
impl Source {
    fn parse(input: &str) -> Result<Self, String> {
        if input.len() > 2048 {
            return Err("GitHub URL exceeds 2048 bytes".into());
        }
        let url = Url::parse(input)
            .map_err(|_| "Expected an HTTPS GitHub repository or Skill directory URL")?;
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Expected a public https://github.com repository URL".into());
        }
        let decoded = url.path_segments().into_iter().flatten();
        let mut parts = Vec::new();
        for part in decoded {
            if part.is_empty() {
                continue;
            }
            // Form decoding treats '+' as space, while a URL path uses it literally.
            let encoded = format!("p={}", part.replace('+', "%2B").replace('&', "%26"));
            let value = url::form_urlencoded::parse(encoded.as_bytes())
                .next()
                .unwrap()
                .1
                .into_owned();
            if !valid_component(&value) {
                return Err("Invalid GitHub path component".into());
            }
            parts.push(value);
        }
        if parts.len() < 2 || !slug(&parts[0]) || !slug(&parts[1]) {
            return Err("Expected a GitHub owner and repository".into());
        }
        let mut revision = "HEAD".to_owned();
        let mut path = String::new();
        if parts.len() > 2 {
            if parts.len() < 4 || !matches!(parts[2].as_str(), "tree" | "blob") {
                return Err("Use a repository, tree, or SKILL.md link".into());
            }
            if parts[2] == "blob" && parts.last().unwrap() != "SKILL.md" {
                return Err("The file link must point to SKILL.md".into());
            }
            revision = parts[3].clone();
            let end = parts.len() - usize::from(parts.last().unwrap() == "SKILL.md");
            path = parts[4..end.max(4)].join("/");
        }
        let repo = parts[1].trim_end_matches(".git");
        if !slug(repo) {
            return Err("Invalid GitHub repository".into());
        }
        Ok(Self {
            owner: parts[0].clone(),
            repo: repo.into(),
            revision,
            path,
        })
    }
    fn link(&self, path: &str) -> Url {
        let mut url = endpoint(
            "https://github.com",
            &[&self.owner, &self.repo, "tree", &self.revision],
        );
        if !path.is_empty() {
            url.path_segments_mut().unwrap().extend(path.split('/'));
        }
        url
    }
    fn package(&self, store: &Path) -> Result<PathBuf, String> {
        if !commit_id(&self.revision) {
            return Err("Expected a pinned GitHub commit".into());
        }
        Ok(store
            .join(&self.owner)
            .join(&self.repo)
            .join(&self.revision)
            .join(&self.path)
            .join(".package"))
    }
}
fn commit_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn source_url(reference: &str) -> Option<String> {
    reference
        .strip_prefix(PACKAGE_REFERENCE)
        .map(|path| format!("{GITHUB_PREFIX}{path}"))
}

pub fn package_for_reference(
    store: &Path,
    reference: &str,
) -> Result<Option<(PathBuf, String)>, String> {
    let Some(url) = source_url(reference) else {
        return Ok(None);
    };
    let source = Source::parse(&url)?;
    Ok(Some((source.package(store)?, source.path)))
}

/// Keep the entire checkout private to this Skill while resolving its upstream
/// relative paths from the original package directory.
pub fn resource_base(root: &Path, manifest: &Value) -> PathBuf {
    let source = manifest["source_url"]
        .as_str()
        .and_then(|reference| Source::parse(reference).ok());
    if let Some(source) = source.filter(|s| commit_id(&s.revision)) {
        let nested = root.join(RESOURCE_TREE).join(source.path);
        if nested.join("SKILL.md").is_file() {
            return nested;
        }
    }
    root.to_owned()
}

#[derive(Deserialize)]
struct Entry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
}
#[derive(Deserialize)]
struct Tree {
    tree: Vec<Entry>,
    truncated: bool,
}

fn rate_limited(response: &reqwest::blocking::Response) -> bool {
    response.status().as_u16() == 429
        || response.status().as_u16() == 403
            && (response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0")
                || response.headers().contains_key("retry-after"))
}

// GitHub repository archives carry the resolved commit in the ZIP comment.
// Read only their index. Installation still downloads and validates the chosen
// regular files from raw.githubusercontent.com at that exact commit.
fn archive_inventory(
    source: &mut Source,
    client: &Client,
    scratch: &Path,
) -> Result<Vec<Entry>, String> {
    const ARCHIVE_LIMIT: u64 = 64 << 20;
    let mut reference = source.revision.clone();
    let mut path = source.path.as_str();
    let response = loop {
        let url = endpoint(
            "https://codeload.github.com",
            &[&source.owner, &source.repo, "zip", &reference],
        );
        let response = client.get(url).send().map_err(|e| e.to_string())?;
        if response.status().as_u16() == 404 && !path.is_empty() && !commit_id(&reference) {
            let (next, rest) = path.split_once('/').unwrap_or((path, ""));
            reference.push('/');
            reference.push_str(next);
            path = rest;
            continue;
        }
        break response.error_for_status().map_err(|e| e.to_string())?;
    };
    if response
        .content_length()
        .is_some_and(|size| size > ARCHIVE_LIMIT)
    {
        return Err("GitHub API quota exhausted and public repository archive exceeds 64 MiB; retry after the quota resets".into());
    }
    // Android applications cannot rely on a writable process-wide temp directory.
    fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::tempfile_in(scratch).map_err(|e| e.to_string())?;
    let count = io::copy(&mut response.take(ARCHIVE_LIMIT + 1), &mut temporary)
        .map_err(|e| e.to_string())?;
    if count > ARCHIVE_LIMIT {
        return Err("GitHub API quota exhausted and public repository archive exceeds 64 MiB; retry after the quota resets".into());
    }
    let (commit, entries) = archive_entries(temporary, &reference)?;
    source.path = path.to_owned();
    source.revision = commit;
    Ok(entries)
}

fn archive_entries(
    reader: impl Read + Seek,
    reference: &str,
) -> Result<(String, Vec<Entry>), String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let commit = std::str::from_utf8(archive.comment())
        .ok()
        .filter(|s| commit_id(s))
        .ok_or("Missing commit ID in GitHub archive")?
        .to_owned();
    if commit_id(reference) && !commit.eq_ignore_ascii_case(reference) {
        return Err("GitHub archive returned a different commit".into());
    }
    if archive.len() > 50000 {
        return Err("GitHub repository archive contains too many entries".into());
    }
    let mut entries = Vec::new();
    let mut root = None;
    for i in 0..archive.len() {
        let file = archive.by_index_raw(i).map_err(|e| e.to_string())?;
        let (prefix, path) = file
            .name()
            .split_once('/')
            .ok_or("Missing GitHub archive root")?;
        if !valid_component(prefix) || root.get_or_insert_with(|| prefix.to_owned()) != prefix {
            return Err("Unexpected GitHub archive root".into());
        }
        if file.is_dir() {
            continue;
        }
        let mode = file.unix_mode().unwrap_or(0);
        let kind = match mode & 0o170000 {
            0 | 0o100000 if mode & 0o111 != 0 => "100755",
            0 | 0o100000 => "100644",
            _ => "unsupported",
        };
        entries.push(Entry {
            path: path.to_owned(),
            kind: "blob".into(),
            mode: kind.into(),
            size: Some(file.size()),
        });
    }
    Ok((commit, entries))
}

// Picking a Skill from a collection returns a pinned URL. Reuse the last
// immutable tree so a phone does not download a second archive for that choice.
struct CachedTree {
    owner: String,
    repo: String,
    revision: String,
    entries: Arc<[Entry]>,
}
static LAST_TREE: Mutex<Option<CachedTree>> = Mutex::new(None);

fn inventory(client: &Client, source: &mut Source, scratch: &Path) -> Result<Arc<[Entry]>, String> {
    if commit_id(&source.revision) {
        if let Ok(cache) = LAST_TREE.lock() {
            if let Some(tree) = cache.as_ref().filter(|t| {
                t.owner == source.owner && t.repo == source.repo && t.revision == source.revision
            }) {
                return Ok(Arc::clone(&tree.entries));
            }
        }
    }
    let entries: Arc<[Entry]> = load_inventory(client, source, scratch)?.into();
    if entries.len() <= 4096 && entries.iter().map(|e| e.path.len()).sum::<usize>() <= (512 << 10) {
        if let Ok(mut cache) = LAST_TREE.lock() {
            *cache = Some(CachedTree {
                owner: source.owner.clone(),
                repo: source.repo.clone(),
                revision: source.revision.clone(),
                entries: Arc::clone(&entries),
            });
        }
    }
    Ok(entries)
}

fn load_inventory(
    client: &Client,
    source: &mut Source,
    scratch: &Path,
) -> Result<Vec<Entry>, String> {
    // GitHub tree links do not delimit a branch name containing slashes.
    // Resolve the ref before interpreting the remaining path as a directory.
    let mut reference = source.revision.clone();
    let mut remaining = source.path.as_str();
    let commit = loop {
        let mut url = endpoint(API, &["repos", &source.owner, &source.repo, "commits"]);
        url.query_pairs_mut()
            .append_pair("sha", &reference)
            .append_pair("per_page", "1");
        let response = client.get(url).send().map_err(|e| e.to_string())?;
        if rate_limited(&response) {
            return archive_inventory(source, client, scratch);
        }
        if matches!(response.status().as_u16(), 404 | 422)
            && !remaining.is_empty()
            && !commit_id(&reference)
        {
            let (next, rest) = remaining.split_once('/').unwrap_or((remaining, ""));
            reference.push('/');
            reference.push_str(next);
            remaining = rest;
            continue;
        }
        let commits: Vec<Value> = serde_json::from_slice(&read_response(response, 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
        break commits
            .into_iter()
            .next()
            .ok_or("GitHub reference has no commits")?;
    };
    source.path = remaining.to_owned();
    let revision = commit["sha"]
        .as_str()
        .filter(|s| commit_id(s))
        .ok_or("Missing GitHub commit")?;
    let tree = commit
        .pointer("/commit/tree/sha")
        .and_then(Value::as_str)
        .filter(|s| commit_id(s))
        .ok_or("Missing GitHub tree")?;
    let mut url = endpoint(
        API,
        &["repos", &source.owner, &source.repo, "git", "trees", tree],
    );
    url.query_pairs_mut().append_pair("recursive", "1");
    source.revision = revision.into();
    let response = client.get(url).send().map_err(|e| e.to_string())?;
    if rate_limited(&response) {
        return archive_inventory(source, client, scratch);
    }
    let result: Tree =
        serde_json::from_slice(&read_response(response, 24 << 20)?).map_err(|e| e.to_string())?;
    if result.truncated {
        return Err("GitHub returned an incomplete repository tree".into());
    }
    Ok(result.tree)
}

fn directories(tree: &[Entry], path: &str) -> Vec<String> {
    let mut result: Vec<_> = tree
        .iter()
        .filter(|e| e.kind == "blob" && matches!(e.mode.as_str(), "100644" | "100755"))
        .filter_map(|e| {
            if e.path == "SKILL.md" {
                Some("")
            } else {
                e.path.strip_suffix("/SKILL.md")
            }
        })
        .filter(|dir| {
            (path.is_empty()
                || *dir == path
                || dir.strip_prefix(path).is_some_and(|s| s.starts_with('/')))
                && (dir.is_empty() || dir.split('/').all(valid_component))
        })
        .map(str::to_owned)
        .collect();
    result.sort();
    result
}
fn choices(source: &Source, dirs: &[String]) -> Value {
    json!({"repository":format!("{}/{}",source.owner,source.repo),"commit":source.revision,
        "skills":dirs.iter().map(|p|json!({"path":p,"url":source.link(p).as_str()})).collect::<Vec<_>>()})
}
pub fn find(query: &str) -> Result<Value, String> {
    find_at(query, &std::env::temp_dir())
}

fn find_at(query: &str, scratch: &Path) -> Result<Value, String> {
    let client = client()?;
    let query = query.trim();
    if query.is_empty() || query.len() > 2048 {
        return Err("Provide a Skill name or GitHub link".into());
    }
    if query.starts_with("https://") {
        let mut source = Source::parse(query)?;
        let tree = inventory(&client, &mut source, scratch)?;
        return Ok(choices(&source, &directories(&tree, &source.path)));
    }
    let mut url = endpoint(API, &["search", "repositories"]);
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("per_page", "8");
    let results: Value =
        serde_json::from_slice(&read_url(&client, url, 1 << 20)?).map_err(|e| e.to_string())?;
    let repos = results["items"]
        .as_array()
        .ok_or("Missing GitHub search results")?;
    Ok(
        json!({"repositories":repos.iter().map(|r|json!({"repository":r["full_name"],"url":r["html_url"],"description":r["description"]})).collect::<Vec<_>>()}),
    )
}

fn raw_file_url(source: &Source, path: &str) -> Url {
    let mut url = endpoint(RAW, &[&source.owner, &source.repo, &source.revision]);
    url.path_segments_mut().unwrap().extend(path.split('/'));
    url
}

fn shared_resources(manifest: &[u8], directory: &str) -> Result<Vec<String>, String> {
    // This optional field names data files. No executable manifest directives
    // or package-manager actions are interpreted.
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_slice(manifest)
        .map_err(|e| format!("Invalid Skill manifest.yaml: {e}"))?;
    let Some(declared) = value.get("always_load") else {
        return Ok(Vec::new());
    };
    let list = declared
        .as_sequence()
        .ok_or("manifest always_load must list file paths")?;
    let mut paths = Vec::new();
    for item in list {
        let relative = item
            .as_str()
            .ok_or("Expected a relative file in manifest always_load")?;
        let mut parts: Vec<_> = directory.split('/').filter(|p| !p.is_empty()).collect();
        for part in relative.split('/') {
            match part {
                "." => (),
                ".." => {
                    parts.pop().ok_or("Skill resource escapes the repository")?;
                }
                other if valid_component(other) => parts.push(other),
                _ => return Err(format!("Invalid required Skill resource: {relative}")),
            }
        }
        paths.push(parts.join("/"));
    }
    Ok(paths)
}

fn fetch_package(
    client: &Client,
    source: &Source,
    tree: &[Entry],
    target: &Path,
) -> Result<(), String> {
    let prefix = if source.path.is_empty() {
        String::new()
    } else {
        format!("{}/", source.path)
    };
    let selected = tree
        .iter()
        .filter(|e| e.kind != "tree" && e.path.starts_with(&prefix))
        .collect::<Vec<_>>();
    let manifest_path = format!("{prefix}manifest.yaml");
    let mut entries = selected;
    if let Some(manifest) = entries.iter().find(|e| e.path == manifest_path) {
        if !matches!(manifest.mode.as_str(), "100644" | "100755") {
            return Err("Unsupported Skill manifest".into());
        }
        let data = read_url(client, raw_file_url(source, &manifest.path), 512 * 1024)?;
        for path in shared_resources(&data, &source.path)? {
            if entries.iter().any(|e| e.path == path) {
                continue;
            }
            let file = tree
                .iter()
                .find(|e| e.path == path && e.kind == "blob")
                .ok_or_else(|| format!("Missing required Skill resource: {path}"))?;
            entries.push(file);
        }
    }
    let mut total = 0u64;
    let mut files = Vec::new();
    for entry in entries {
        let rel = entry.path.as_str();
        if entry.kind != "blob"
            || !matches!(entry.mode.as_str(), "100644" | "100755")
            || !rel.split('/').all(valid_component)
        {
            return Err(format!("Unsupported file in Skill package: {rel}"));
        }
        let size = entry.size.ok_or("Missing GitHub file size")?;
        total = total.checked_add(size).ok_or("Package size overflow")?;
        if total > PACKAGE_LIMIT || files.len() >= 1024 {
            return Err("Skill package exceeds 32 MiB or 1024 files".into());
        }
        files.push((entry, rel, size));
    }
    for chunk in files.chunks(8) {
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for &(entry, rel, size) in chunk {
                workers.push(scope.spawn(move || -> Result<(), String> {
                    let data = read_url(client, raw_file_url(source, &entry.path), size)?;
                    if data.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                        return Err(format!("Skill file requires Git LFS: {rel}"));
                    }
                    if data.len() as u64 != size {
                        return Err(format!("Incomplete Skill file: {rel}"));
                    }
                    let file = target.join(rel);
                    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
                    use std::io::Write;
                    let mut output = fs::File::create_new(&file).map_err(|e| e.to_string())?;
                    output.write_all(&data).map_err(|e| e.to_string())?;
                    #[cfg(unix)]
                    if entry.mode == "100755" {
                        use std::os::unix::fs::PermissionsExt;
                        output
                            .set_permissions(fs::Permissions::from_mode(0o755))
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                }));
            }
            for worker in workers {
                worker.join().map_err(|_| "GitHub download interrupted")??;
            }
            Ok::<(), String>(())
        })?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct PreparedPackage {
    pub id: String,
    pub description: String,
    pub content: String,
    pub version: String,
    pub instructions_ref: String,
    pub source_url: String,
    pub base_directory: PathBuf,
    pub package_root: PathBuf,
}

pub fn prepare_with(
    store: &Path,
    input: &str,
    register: impl FnOnce(PreparedPackage) -> Result<Value, String>,
) -> Result<Value, String> {
    let client = client()?;
    let mut source = Source::parse(input)?;
    let tree = inventory(&client, &mut source, store)?;
    let dirs = directories(&tree, &source.path);
    if !dirs.contains(&source.path) {
        match dirs.as_slice() {
            [] => return Err("No SKILL.md found at the selected GitHub location".into()),
            [only] => source.path = only.clone(),
            _ => {
                let mut result = choices(&source, &dirs);
                result["status"] = json!("selection_required");
                return Ok(result);
            }
        }
    }
    let destination = source.package(store)?;
    let origin = source.link(&source.path).to_string();
    let reference = origin.replacen(GITHUB_PREFIX, PACKAGE_REFERENCE, 1);
    let parent = destination.parent().unwrap();
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = if destination.is_dir() {
        None
    } else {
        let stage = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
        fetch_package(&client, &source, &tree, stage.path())?;
        Some(stage)
    };
    let directory = stage
        .as_ref()
        .map_or(destination.as_path(), |stage| stage.path());
    let (id, description, text) = read_metadata(&directory.join(&source.path))?;
    // Concurrent installers at the same immutable source share one completed
    // directory and one create/enable decision.
    let publication_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install-lock"))
        .map_err(|e| e.to_string())?;
    publication_lock.lock().map_err(|e| e.to_string())?;
    if let Some(stage) = stage {
        if !destination.is_dir() {
            fs::rename(stage.path(), &destination).map_err(|e| e.to_string())?;
        }
    }
    register(PreparedPackage {
        id,
        description,
        content: text,
        version: source.revision,
        instructions_ref: reference,
        source_url: origin,
        base_directory: destination.join(source.path),
        package_root: destination,
    })
}

pub fn markdown_frontmatter_body(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let marker = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"))?;
    let body = &rest[..marker];
    let content_start = marker
        + if rest[marker..].starts_with("\r\n---\r\n") {
            7
        } else {
            5
        };
    Some((body, &rest[content_start..]))
}

fn read_metadata(path: &Path) -> Result<(String, String, String), String> {
    let text = fs::read_to_string(path.join("SKILL.md")).map_err(|e| e.to_string())?;
    if text.chars().count() > 200_000 {
        return Err("Skill instructions exceed 200000 characters".into());
    }
    #[derive(Deserialize)]
    struct Frontmatter {
        name: String,
        description: String,
    }
    let (header, _) =
        markdown_frontmatter_body(&text).ok_or("Missing SKILL.md YAML front matter")?;
    let metadata: Frontmatter = serde_yaml_ng::from_str(header)
        .map_err(|e| format!("Invalid SKILL.md front matter: {e}"))?;
    if !slug(&metadata.name) || metadata.name.len() > 128 || metadata.description.trim().is_empty()
    {
        return Err("SKILL.md needs a valid name and description".into());
    }
    Ok((metadata.name, metadata.description.trim().to_owned(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, mode: &str, size: u64) -> Entry {
        Entry {
            path: path.into(),
            kind: "blob".into(),
            mode: mode.into(),
            size: Some(size),
        }
    }

    #[test]
    #[ignore = "downloads the public nature-skills archive and selected package"]
    fn public_archive_installs_a_pinned_skill_with_shared_resources() {
        let root = tempfile::tempdir().unwrap();
        let mut source = Source::parse(
            "https://github.com/Yuan1z0825/nature-skills/tree/main/skills/nature-paper-card",
        )
        .unwrap();
        let client = client().unwrap();
        let tree = archive_inventory(&mut source, &client, root.path()).unwrap();
        assert!(commit_id(&source.revision));
        assert_eq!(directories(&tree, &source.path), [source.path.as_str()]);
        let package = root.path().join("package");
        fetch_package(&client, &source, &tree, &package).unwrap();
        assert_eq!(
            read_metadata(&package.join(source.path)).unwrap().0,
            "nature-paper-card"
        );
        assert!(package
            .join("skills/nature-shared/core/terminology-ledger.md")
            .is_file());
    }

    #[test]
    fn archive_index_requires_matching_commit_and_rejects_symlinks_as_skills() {
        use std::io::Cursor;
        const PIN: &str = "0123456789abcdef0123456789abcdef01234567";
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive.set_comment(PIN).unwrap();
        archive
            .start_file("repo-main/card/SKILL.md", options)
            .unwrap();
        archive
            .add_symlink("repo-main/link/SKILL.md", "../../elsewhere", options)
            .unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let (commit, tree) = archive_entries(Cursor::new(&bytes), "main").unwrap();
        assert_eq!(commit, PIN);
        assert_eq!(directories(&tree, ""), ["card"]);
        assert!(archive_entries(Cursor::new(&bytes), &"a".repeat(40)).is_err());
    }

    #[test]
    fn accepts_repository_directory_and_document_links() {
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git/",
        ] {
            let source = Source::parse(url).unwrap();
            assert_eq!(
                (&*source.repo, &*source.revision, &*source.path),
                ("repo", "HEAD", "")
            );
        }
        let source =
            Source::parse("https://github.com/owner/repo/blob/release/skills/a%20b/SKILL.md")
                .unwrap();
        assert_eq!(source.path, "skills/a b");
        assert_eq!(
            source.link(&source.path).as_str(),
            "https://github.com/owner/repo/tree/release/skills/a%20b"
        );
        for url in [
            "http://github.com/a/b",
            "https://other.example/a/b",
            "https://u@github.com/a/b",
            "https://github.com/a/b/blob/main/README.md",
            "https://github.com/a/b/tree/main/a%2Fb",
        ] {
            assert!(Source::parse(url).is_err(), "{url}");
        }
    }

    #[test]
    fn lists_only_installable_packages_under_selected_path() {
        let entries = vec![
            entry("SKILL.md", "100644", 1),
            entry("a/one/SKILL.md", "100644", 1),
            entry("ab/two/SKILL.md", "100644", 1),
            entry("a/link/SKILL.md", "120000", 1),
        ];
        assert_eq!(directories(&entries, "a"), ["a/one"]);
        assert_eq!(directories(&entries, ""), ["", "a/one", "ab/two"]);
    }

    #[test]
    fn rejects_unsupported_files_before_downloading_anything() {
        let root = tempfile::tempdir().unwrap();
        let source = Source::parse("https://github.com/owner/repo/tree/main/test").unwrap();
        for bad in [
            entry("test/../escape", "100644", 12),
            entry("test/symlink", "120000", 1),
            entry("test/NOLA_MANIFEST.json", "100644", 12),
            entry("test/.nola-package/asset", "100644", 12),
            entry("test/oversize", "100644", PACKAGE_LIMIT + 1),
        ] {
            let entries = vec![entry("test/SKILL.md", "100644", 14), bad];
            assert!(fetch_package(&client().unwrap(), &source, &entries, root.path()).is_err());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn manifest_files_keep_their_repository_relative_locations() {
        let yaml = b"always_load:\n  - ../shared/ledger.md\n  - 'static/core.md'\n";
        assert_eq!(
            shared_resources(yaml, "skills/card").unwrap(),
            ["skills/shared/ledger.md", "skills/card/static/core.md"]
        );
        for file in [
            "../../../outside",
            "/absolute",
            "https://other.test/file",
            "../.git/index",
        ] {
            let yaml = format!("always_load: ['{file}']");
            assert!(shared_resources(yaml.as_bytes(), "skills/card").is_err());
        }
    }

    #[test]
    fn requires_frontmatter_identity_and_description() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("SKILL.md");
        fs::write(
            &file,
            "---\nname: sample\ndescription: Sample package\n---\nInstructions",
        )
        .unwrap();
        let (id, desc, _) = read_metadata(root.path()).unwrap();
        assert_eq!((id.as_str(), desc.as_str()), ("sample", "Sample package"));
        for text in [
            "# sample",
            "---\nname: ..\ndescription: Sample\n---",
            "---\nname: sample\n---",
        ] {
            fs::write(&file, text).unwrap();
            assert!(read_metadata(root.path()).is_err());
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum MobileOperation {
    Find {
        query: String,
    },
    Prepare {
        url: String,
    },
    Resolve {
        instructions_ref: String,
    },
    Read {
        instructions_ref: String,
        path: String,
    },
}

fn installed_directories(store: &Path, reference: &str) -> Result<(PathBuf, PathBuf), String> {
    let (package, relative) = package_for_reference(store, reference)?
        .ok_or("Expected a registered GitHub Skill reference")?;
    let root = store
        .canonicalize()
        .map_err(|_| "Skill package store is unavailable")?;
    let package = package
        .canonicalize()
        .map_err(|_| "Skill package is missing")?;
    let directory = package
        .join(relative)
        .canonicalize()
        .map_err(|_| "Skill directory is missing")?;
    if !package.starts_with(root)
        || !directory.starts_with(&package)
        || !directory.join("SKILL.md").is_file()
    {
        return Err("Invalid installed Skill directory".into());
    }
    Ok((package, directory))
}

fn run_mobile(store: &Path, input: &str) -> Result<Value, String> {
    if !store.is_absolute() || input.len() > 32 * 1024 {
        return Err("Invalid Skill Host request".into());
    }
    match serde_json::from_str(input).map_err(|e| format!("Invalid Skill request: {e}"))? {
        MobileOperation::Find { query } => find_at(&query, store),
        MobileOperation::Prepare { url } => prepare_with(store, &url, |record| {
            let mut value = serde_json::to_value(record).map_err(|e| e.to_string())?;
            value["status"] = json!("prepared");
            Ok(value)
        }),
        MobileOperation::Resolve { instructions_ref } => {
            let (package, directory) = installed_directories(store, &instructions_ref)?;
            Ok(json!({"package_root":package,"base_directory":directory}))
        }
        MobileOperation::Read {
            instructions_ref,
            path,
        } => {
            let (package, directory) = installed_directories(store, &instructions_ref)?;
            if path.len() > 2048
                || path
                    .split('/')
                    .any(|p| !matches!(p, "." | "..") && !valid_component(p))
            {
                return Err("Expected a relative Skill resource path".into());
            }
            let file = directory
                .join(&path)
                .canonicalize()
                .map_err(|_| "Skill resource is unavailable")?;
            if !file.starts_with(&package) || !file.is_file() {
                return Err("Path is outside this Skill package".into());
            }
            let mut bytes = Vec::new();
            let reader = fs::File::open(file).map_err(|e| e.to_string())?;
            const READ_LIMIT: u64 = 256 * 1024;
            reader
                .take(READ_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > READ_LIMIT {
                return Err("Skill resource exceeds 256 KiB".into());
            }
            let content =
                String::from_utf8(bytes).map_err(|_| "Skill resource is not UTF-8 text")?;
            Ok(json!({"path":path,"content":content}))
        }
    }
}

/// The caller owns the private store path and registers the resulting Skill
/// through its existing user, revision and access controls.
pub fn mobile_request(store: &Path, input: &str) -> String {
    match run_mobile(store, input) {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    }
    .to_string()
}

#[cfg(test)]
mod mobile_tests {
    use super::*;
    #[test]
    fn reads_declared_sibling_only_inside_pinned_package() {
        let store = tempfile::tempdir().unwrap();
        let reference = format!(
            "{PACKAGE_REFERENCE}owner/repo/tree/{}/skills/card",
            "a".repeat(40)
        );
        let (root, relative) = package_for_reference(store.path(), &reference)
            .unwrap()
            .unwrap();
        let base = root.join(relative);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("SKILL.md"), "# Card").unwrap();
        fs::create_dir_all(root.join("skills/shared")).unwrap();
        fs::write(root.join("skills/shared/sample.md"), "Shared content").unwrap();
        fs::write(root.parent().unwrap().join("private"), "outside").unwrap();
        let read = |path: &str| {
            run_mobile(
                store.path(),
                &json!({"operation":"read","instructions_ref":reference,"path":path}).to_string(),
            )
        };
        assert_eq!(
            read("../shared/sample.md").unwrap()["content"],
            "Shared content"
        );
        assert!(read("../../../private").is_err());
        assert!(read("/etc/hosts").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.parent().unwrap().join("private"), base.join("linked"))
                .unwrap();
            assert!(read("linked").is_err());
        }
    }
}
