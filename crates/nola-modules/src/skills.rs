// SPDX-License-Identifier: Apache-2.0
//! Skills discovery (`SKILL.md` with YAML frontmatter + optional `NOLA_MANIFEST.json`),
//! persistent enable/disable state (`kv` `skill_disabled:{name}`), GitHub skill package
//! preparation, and tail-safe `skill_load` tool.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use nola_store::{Store, DEFAULT_MAX_EVENTS_PER_SCOPE};
use nola_tools::{path_within_workspace, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, SkillDescriptor, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

const MAX_SKILL_DOWNLOAD_BYTES: u64 = 256 * 1024;
const MAX_SUPPORTING_SKILL_FILES: usize = 16;
pub const KIND_SKILL_SKIP: &str = "skill_skip";

pub fn skill_disabled_kv_key(name: &str) -> String {
    format!("skill_disabled:{}", name.trim().to_ascii_lowercase())
}

/// Check whether a skill is enabled in the store (defaults to `true`).
pub fn is_skill_enabled(store: &Store, name: &str) -> bool {
    let key = skill_disabled_kv_key(name);
    match store.get_kv(&key).ok().flatten() {
        Some(Value::Bool(disabled)) => !disabled,
        _ => true,
    }
}

/// Persist a skill's enabled/disabled toggle under `kv` key `skill_disabled:{name}`.
pub fn set_skill_enabled(
    store: &Store,
    name: &str,
    enabled: bool,
) -> Result<(), nola_store::StoreError> {
    let key = skill_disabled_kv_key(name);
    store.set_kv(&key, &Value::Bool(!enabled))
}

pub fn builtin_skills() -> Vec<SkillDescriptor> {
    vec![
        SkillDescriptor {
            name: "code-review".into(),
            description: "Systematic security, correctness, and maintainability review workflow".into(),
            path: "builtin://code-review".into(),
            body: "1. Read git diff and surrounding code context.\n2. Check invariants, error handling, security boundaries, and concurrency.\n3. Report concrete file:line findings ordered by severity.".into(),
        },
        SkillDescriptor {
            name: "debugging".into(),
            description: "Root-cause debugging workflow with minimal reproduction before patching".into(),
            path: "builtin://debugging".into(),
            body: "1. Reproduce the failure deterministically.\n2. Form a hypothesis and verify with targeted logs/tests.\n3. Apply the minimal root-cause fix and add a regression test.".into(),
        },
        SkillDescriptor {
            name: "tdd".into(),
            description: "Test-driven development loop (red -> green -> refactor)".into(),
            path: "builtin://tdd".into(),
            body: "1. Write a failing test defining the expected contract.\n2. Implement the cleanest code to make it pass.\n3. Refactor while keeping tests green.".into(),
        },
    ]
}

fn read_skill_manifest(skill_file: &Path) -> Option<Value> {
    let parent = skill_file.parent()?;
    let manifest_path = parent.join("NOLA_MANIFEST.json");
    let raw = fs::read_to_string(manifest_path).ok()?;
    serde_json::from_str::<Value>(&raw).ok()
}

pub fn discover_skills(workspace_root: &Path, user_home: Option<&Path>) -> Vec<SkillDescriptor> {
    let mut roots = vec![workspace_root.join(".nola/skills")];
    if let Some(home) = user_home {
        roots.push(home.join(".nola/skills"));
    }
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let skill_file = if p.is_dir() {
                p.join("SKILL.md")
            } else if p.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
                p
            } else {
                continue;
            };
            if let Ok(raw) = fs::read_to_string(&skill_file) {
                if let Some(mut desc) = parse_skill_md(&skill_file, &raw) {
                    if let Some(manifest) = read_skill_manifest(&skill_file) {
                        if let Some(m_name) = manifest
                            .get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                        {
                            desc.name = m_name.to_string();
                        }
                        if desc.description.is_empty() {
                            if let Some(m_desc) =
                                manifest.get("description").and_then(|v| v.as_str())
                            {
                                desc.description = m_desc.to_string();
                            }
                        }
                    }
                    if !out.iter().any(|s: &SkillDescriptor| s.name == desc.name) {
                        out.push(desc);
                    }
                }
            }
        }
    }
    for builtin in builtin_skills() {
        if !out.iter().any(|s| s.name == builtin.name) {
            out.push(builtin);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Discover skills with rich metadata including `base_directory`, `enabled` state
/// (persisted in `kv` `skill_disabled:{name}`), and parsed `NOLA_MANIFEST.json`.
pub fn discover_skills_detailed(
    store: Option<&Store>,
    workspace_root: &Path,
    user_home: Option<&Path>,
) -> Vec<Value> {
    discover_skills(workspace_root, user_home)
        .into_iter()
        .map(|s| {
            let is_builtin = s.path.starts_with("builtin://");
            let base_directory = if is_builtin {
                Value::Null
            } else {
                Path::new(&s.path)
                    .parent()
                    .map(|p| json!(p.display().to_string()))
                    .unwrap_or(Value::Null)
            };
            let manifest = if is_builtin {
                Value::Null
            } else {
                read_skill_manifest(Path::new(&s.path)).unwrap_or(Value::Null)
            };
            let enabled = store
                .map(|st| is_skill_enabled(st, &s.name))
                .unwrap_or(true);
            json!({
                "name": s.name,
                "description": s.description,
                "path": s.path,
                "base_directory": base_directory,
                "enabled": enabled,
                "source": if is_builtin { "builtin" } else { "discovered" },
                "manifest": manifest,
                "body": s.body
            })
        })
        .collect()
}

fn is_valid_github_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Parse and validate a GitHub skill reference (`owner/repo`, `owner/repo/path/SKILL.md`,
/// `https://github.com/owner/repo/...`, or `https://raw.githubusercontent.com/...`) and prepare
/// its installation descriptor and destination directory inside `skills_dir` for `nola skills install` & mobile FFI.
pub fn prepare_github_skill_package(
    query_or_url: &str,
    skills_dir: &Path,
) -> Result<Value, String> {
    let trimmed = query_or_url.trim();
    if trimmed.is_empty() {
        return Err("skill package reference must not be empty".into());
    }

    let (owner, repo, branch, skill_path): (String, String, String, String) =
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            let parsed = url::Url::parse(trimmed)
                .map_err(|e| format!("invalid GitHub skill URL: {e}"))?;
            if parsed.scheme() != "https" {
                return Err("GitHub skill URL must use https://".into());
            }
            let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
            let segs: Vec<&str> = parsed
                .path_segments()
                .map(|s| s.filter(|p| !p.is_empty()).collect())
                .unwrap_or_default();
            if host == "github.com" || host == "www.github.com" {
                if segs.len() < 2 {
                    return Err("GitHub URL must include owner/repo".into());
                }
                let owner = segs[0].to_string();
                let repo = segs[1].trim_end_matches(".git").to_string();
                let (branch, path) = if segs.len() >= 4 && (segs[2] == "blob" || segs[2] == "tree")
                {
                    let br = segs[3].to_string();
                    let rest = if segs.len() > 4 {
                        segs[4..].join("/")
                    } else {
                        "SKILL.md".to_string()
                    };
                    (
                        br,
                        if rest.ends_with(".md") {
                            rest
                        } else {
                            format!("{}/SKILL.md", rest.trim_end_matches('/'))
                        },
                    )
                } else {
                    ("HEAD".to_string(), "SKILL.md".to_string())
                };
                (owner, repo, branch, path)
            } else if host == "raw.githubusercontent.com" {
                if segs.len() < 4 {
                    return Err(
                        "raw.githubusercontent.com URL must include owner/repo/branch/path".into(),
                    );
                }
                (
                    segs[0].to_string(),
                    segs[1].to_string(),
                    segs[2].to_string(),
                    segs[3..].join("/"),
                )
            } else {
                return Err(format!(
                    "unsupported skill host '{host}'; expected github.com or raw.githubusercontent.com"
                ));
            }
        } else {
            let segs: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
            if segs.len() < 2 {
                return Err("skill spec must be 'owner/repo' or a GitHub URL".into());
            }
            let owner = segs[0].to_string();
            let repo = segs[1].to_string();
            let path = if segs.len() == 2 {
                "SKILL.md".to_string()
            } else {
                let tail = segs[2..].join("/");
                if tail.ends_with(".md") {
                    tail
                } else {
                    format!("{}/SKILL.md", tail.trim_end_matches('/'))
                }
            };
            (owner, repo, "HEAD".to_string(), path)
        };

    if !is_valid_github_segment(&owner)
        || !is_valid_github_segment(&repo)
        || !is_valid_github_segment(&branch)
    {
        return Err("invalid GitHub owner, repo, or branch segment".into());
    }
    let clean_path = skill_path.trim_start_matches('/');
    if clean_path.is_empty()
        || clean_path.contains('?')
        || clean_path.contains('#')
        || clean_path.contains('\\')
        || !clean_path.split('/').all(is_valid_github_segment)
    {
        return Err("invalid skill_path segment or traversal forbidden".into());
    }

    let derived_name = Path::new(clean_path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&repo);
    let safe_name: String = derived_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_name.is_empty() {
        return Err("could not derive valid local skill name".into());
    }

    let raw_url = format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{clean_path}"
    );
    let manifest_rel = match Path::new(clean_path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            format!(
                "{}/NOLA_MANIFEST.json",
                parent.to_string_lossy().replace('\\', "/")
            )
        }
        _ => "NOLA_MANIFEST.json".to_string(),
    };
    let manifest_url = format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{manifest_rel}"
    );
    let dest_dir = skills_dir.join(&safe_name);
    let dest_file = dest_dir.join("SKILL.md");

    Ok(json!({
        "name": safe_name,
        "repo": format!("{owner}/{repo}"),
        "owner": owner,
        "repository": repo,
        "branch": branch,
        "skill_path": clean_path,
        "raw_url": raw_url,
        "manifest_url": manifest_url,
        "base_directory": dest_dir.display().to_string(),
        "dest_file": dest_file.display().to_string()
    }))
}

fn parse_skill_md(path: &Path, raw: &str) -> Option<SkillDescriptor> {
    let fallback_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();

    let normalized = raw.replace("\r\n", "\n");
    let mut name = fallback_name;
    let mut description = String::new();
    let mut body = normalized.trim().to_string();

    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some((fm, tail)) = rest.split_once("\n---\n") {
            for line in fm.lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !v.is_empty() {
                        name = v.to_string();
                    }
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().trim_matches('"').trim_matches('\'').to_string();
                }
            }
            body = tail.trim().to_string();
        }
    }

    Some(SkillDescriptor {
        name,
        description,
        path: path.display().to_string(),
        body,
    })
}

pub fn validate_skill_relative_file_path(rel: &str) -> Result<PathBuf, String> {
    let normalized = rel.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains('?')
        || normalized.contains('#')
    {
        return Err(format!("invalid supporting skill file path '{rel}'"));
    }
    let mut clean = PathBuf::new();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || !is_valid_github_segment(seg) {
            return Err(format!(
                "unsafe path segment '{seg}' in supporting skill file '{rel}'"
            ));
        }
        clean.push(seg);
    }
    if clean.as_os_str().is_empty()
        || clean
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("invalid relative skill file path '{rel}'"));
    }
    Ok(clean)
}

/// Extract and safely validate optional `files` list from `NOLA_MANIFEST.json`
/// (enforcing path traversal checks and the 16-file limit).
pub fn extract_manifest_supporting_files(manifest: &Value) -> Result<Vec<String>, String> {
    let Some(arr) = manifest.get("files").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    if arr.len() > MAX_SUPPORTING_SKILL_FILES {
        return Err(format!(
            "NOLA_MANIFEST.json declares {} supporting files (max {MAX_SUPPORTING_SKILL_FILES})",
            arr.len()
        ));
    }
    let mut out = Vec::new();
    for entry in arr {
        let Some(raw) = entry.as_str() else {
            return Err("NOLA_MANIFEST.json 'files' entries must be strings".into());
        };
        let validated = validate_skill_relative_file_path(raw)?;
        let canonical = validated.to_string_lossy().replace('\\', "/");
        if canonical != "SKILL.md" && canonical != "NOLA_MANIFEST.json" && !out.contains(&canonical)
        {
            out.push(canonical);
        }
    }
    Ok(out)
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(SkillLoadTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(SkillSkipTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(GithubSkillTool {
        suite: Arc::clone(suite),
    }));
}

struct GithubSkillTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for GithubSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "github_skill".into(),
                description: "Search GitHub for Nola SKILL.md repositories or install a skill into .nola/skills (action: find|install).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["find", "install"] },
                        "query": { "type": "string", "description": "Search keywords (for action=find)" },
                        "repo": { "type": "string", "description": "owner/repo (for action=install)" },
                        "skill_path": { "type": "string", "description": "Path to SKILL.md in repo (default SKILL.md)" },
                        "name": { "type": "string", "description": "Local skill directory name" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Network,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if !ctx.network_allowed {
            return ToolExecutionResult::err_structured(
                "network_denied",
                "github_skill blocked by network policy",
                None,
            );
        }
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Nola/2.0")
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::err_structured("http_error", e.to_string(), None),
        };
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("find") {
            "find" => {
                let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("SKILL.md");
                let resp = client
                    .get("https://api.github.com/search/repositories")
                    .query(&[("q", format!("{q} skill")), ("per_page", "10".into())])
                    .send();
                match resp.and_then(|r| r.json::<Value>()) {
                    Ok(body) => {
                        let items: Vec<Value> = body
                            .get("items")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .map(|r| {
                                        json!({
                                            "repo": r["full_name"],
                                            "description": r["description"],
                                            "stars": r["stargazers_count"],
                                            "url": r["html_url"]
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let count = items.len();
                        ToolExecutionResult::ok_structured(
                            json!({"count": count, "repositories": items}),
                            format!("Found {count} GitHub skill repo(s)"),
                        )
                    }
                    Err(e) => ToolExecutionResult::err_structured("github_error", e.to_string(), None),
                }
            }
            "install" => {
                let Some(repo) = args.get("repo").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'repo' (owner/name)", None);
                };
                let parts: Vec<&str> = repo.split('/').collect();
                if parts.len() != 2
                    || !is_valid_github_segment(parts[0])
                    || !is_valid_github_segment(parts[1])
                {
                    return ToolExecutionResult::err_structured(
                        "invalid_repo",
                        "repo must strictly be 'owner/name' using [A-Za-z0-9._-]",
                        None,
                    );
                }
                let skill_path = args
                    .get("skill_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("SKILL.md")
                    .trim_start_matches('/');
                if skill_path.is_empty()
                    || skill_path.contains('?')
                    || skill_path.contains('#')
                    || skill_path.contains('\\')
                    || !skill_path.split('/').all(is_valid_github_segment)
                {
                    return ToolExecutionResult::err_structured(
                        "invalid_path",
                        "invalid skill_path segment or traversal forbidden",
                        None,
                    );
                }
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(parts[1]);
                let safe_name: String = name
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if safe_name.is_empty() {
                    return ToolExecutionResult::err_structured("invalid_name", "invalid skill name", None);
                }
                let raw_url = format!(
                    "https://raw.githubusercontent.com/{}/{}/HEAD/{skill_path}",
                    parts[0], parts[1]
                );
                match client.get(&raw_url).send() {
                    Ok(resp) if resp.status().is_success() => {
                        let mut buf = Vec::new();
                        if let Err(e) = resp
                            .take(MAX_SKILL_DOWNLOAD_BYTES + 1)
                            .read_to_end(&mut buf)
                        {
                            return ToolExecutionResult::err_structured("download_failed", e.to_string(), None);
                        }
                        if buf.len() as u64 > MAX_SKILL_DOWNLOAD_BYTES {
                            return ToolExecutionResult::err_structured(
                                "download_too_large",
                                format!("skill exceeds {MAX_SKILL_DOWNLOAD_BYTES} byte limit"),
                                None,
                            );
                        }
                        let dest_dir = self.suite.workspace_root.join(".nola/skills").join(&safe_name);
                        if let Err(e) = fs::create_dir_all(&dest_dir) {
                            return ToolExecutionResult::err_structured("write_error", e.to_string(), None);
                        }
                        let dest = dest_dir.join("SKILL.md");
                        if let Err(e) = fs::write(&dest, &buf) {
                            return ToolExecutionResult::err_structured("write_error", e.to_string(), None);
                        }

                        // Optionally fetch NOLA_MANIFEST.json and download declared supporting files
                        // safely (path traversal check + 256 KiB per file + max 16 files).
                        let base_rel = Path::new(skill_path)
                            .parent()
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default();
                        let manifest_rel = if base_rel.is_empty() {
                            "NOLA_MANIFEST.json".to_string()
                        } else {
                            format!("{base_rel}/NOLA_MANIFEST.json")
                        };
                        let manifest_url = format!(
                            "https://raw.githubusercontent.com/{}/{}/HEAD/{manifest_rel}",
                            parts[0], parts[1]
                        );
                        let mut installed_files: Vec<String> = Vec::new();
                        let mut manifest_loaded = false;
                        if let Ok(m_resp) = client.get(&manifest_url).send() {
                            if m_resp.status().is_success() {
                                let mut m_buf = Vec::new();
                                if m_resp
                                    .take(MAX_SKILL_DOWNLOAD_BYTES + 1)
                                    .read_to_end(&mut m_buf)
                                    .is_ok()
                                    && (m_buf.len() as u64) <= MAX_SKILL_DOWNLOAD_BYTES
                                {
                                    if let Ok(manifest_val) = serde_json::from_slice::<Value>(&m_buf)
                                    {
                                        let _ = fs::write(dest_dir.join("NOLA_MANIFEST.json"), &m_buf);
                                        manifest_loaded = true;
                                        match extract_manifest_supporting_files(&manifest_val) {
                                            Ok(rel_files) => {
                                                for rel in rel_files {
                                                    let Ok(clean_rel) =
                                                        validate_skill_relative_file_path(&rel)
                                                    else {
                                                        continue;
                                                    };
                                                    let target = dest_dir.join(&clean_rel);
                                                    if !path_within_workspace(&dest_dir, &target) {
                                                        continue;
                                                    }
                                                    let file_url = if base_rel.is_empty() {
                                                        format!(
                                                            "https://raw.githubusercontent.com/{}/{}/HEAD/{rel}",
                                                            parts[0], parts[1]
                                                        )
                                                    } else {
                                                        format!(
                                                            "https://raw.githubusercontent.com/{}/{}/HEAD/{base_rel}/{rel}",
                                                            parts[0], parts[1]
                                                        )
                                                    };
                                                    if let Ok(f_resp) = client.get(&file_url).send() {
                                                        if f_resp.status().is_success() {
                                                            let mut f_buf = Vec::new();
                                                            if f_resp
                                                                .take(MAX_SKILL_DOWNLOAD_BYTES + 1)
                                                                .read_to_end(&mut f_buf)
                                                                .is_ok()
                                                                && (f_buf.len() as u64)
                                                                    <= MAX_SKILL_DOWNLOAD_BYTES
                                                            {
                                                                if let Some(parent) = target.parent() {
                                                                    let _ = fs::create_dir_all(parent);
                                                                }
                                                                if fs::write(&target, &f_buf).is_ok() {
                                                                    installed_files.push(rel);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                return ToolExecutionResult::err_structured(
                                                    "invalid_manifest_files",
                                                    err,
                                                    None,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        ToolExecutionResult::ok_structured(
                            json!({
                                "installed": true,
                                "name": safe_name,
                                "path": dest.display().to_string(),
                                "base_directory": dest_dir.display().to_string(),
                                "manifest_loaded": manifest_loaded,
                                "supporting_files": installed_files
                            }),
                            format!("Installed skill '{safe_name}' from {repo}"),
                        )
                    }
                    Ok(resp) => ToolExecutionResult::err_structured(
                        "download_failed",
                        format!("GitHub raw fetch returned HTTP {}", resp.status()),
                        None,
                    ),
                    Err(e) => ToolExecutionResult::err_structured("github_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured("invalid_action", format!("unsupported github_skill action '{other}'"), None),
        }
    }
}

struct SkillLoadTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for SkillLoadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "skill_load".into(),
                description: "List available skills or load a skill's instructions (injected at tail without invalidating prompt cache).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Skill name to load; omit to list all discovered skills" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let detailed = discover_skills_detailed(
            Some(&self.suite.store),
            &self.suite.workspace_root,
            self.suite.user_home.as_deref(),
        );
        let Some(name) = args
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            let list: Vec<Value> = detailed
                .into_iter()
                .map(|s| {
                    json!({
                        "name": s["name"],
                        "description": s["description"],
                        "path": s["path"],
                        "base_directory": s["base_directory"],
                        "enabled": s["enabled"],
                        "manifest": s["manifest"]
                    })
                })
                .collect();
            let count = list.len();
            return ToolExecutionResult::ok_structured(
                json!({"count": count, "skills": list}),
                format!("Discovered {count} skill(s)"),
            );
        };
        match detailed.into_iter().find(|s| {
            s.get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        }) {
            Some(skill) => {
                if skill.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                    return ToolExecutionResult::err_structured(
                        "skill_disabled",
                        format!("skill '{name}' is disabled"),
                        None,
                    );
                }
                let sname = skill["name"].as_str().unwrap_or(name).to_string();
                ToolExecutionResult::ok_structured(skill, format!("Loaded skill '{sname}'"))
            }
            None => ToolExecutionResult::err_structured(
                "skill_not_found",
                format!("skill '{name}' not found"),
                None,
            ),
        }
    }
}

/// Explicitly record why an advertised skill was skipped for the current task (`skill_skip`).
pub struct SkillSkipTool {
    suite: Arc<ModulesSuite>,
}

impl SkillSkipTool {
    pub fn new(suite: Arc<ModulesSuite>) -> Self {
        Self { suite }
    }
}

impl ToolHandler for SkillSkipTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "skill_skip".into(),
                description: "Explicitly record why an advertised skill was skipped for the current task.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Advertised skill name being skipped" },
                        "reason": { "type": "string", "description": "Concise rationale for skipping the skill" },
                        "session_id": { "type": "string", "description": "Optional session ID (defaults to current session)" }
                    },
                    "required": ["name", "reason"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(name) = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'name'", None);
        };
        let Some(reason) = args
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'reason'", None);
        };
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(ctx.session_id.as_str());
        let ts = now_ms();
        let skip_id = new_id();
        let entry = json!({
            "id": skip_id,
            "name": name,
            "reason": reason,
            "session_id": session_id,
            "skipped_at_ms": ts
        });
        let _ = self.suite.upsert_record(KIND_SKILL_SKIP, &skip_id, &entry);
        let _ = self.suite.store.append_event(
            session_id,
            "skill_skipped",
            entry.clone(),
            DEFAULT_MAX_EVENTS_PER_SCOPE,
        );
        ToolExecutionResult::ok_structured(
            entry,
            format!("Recorded skip for skill '{name}': {reason}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_parse_manifest_persist_enabled_and_prepare_github_package() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(".nola/skills/release");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: release\n---\nRun cargo test before tagging.\n",
        )
        .unwrap();
        fs::write(
            skill_dir.join("NOLA_MANIFEST.json"),
            r#"{"name":"release","description":"Release checklist from manifest","version":"1.0.0"}"#,
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        assert!(is_skill_enabled(&store, "release"));
        set_skill_enabled(&store, "release", false).unwrap();
        assert!(!is_skill_enabled(&store, "release"));

        let detailed = discover_skills_detailed(Some(&store), dir.path(), None);
        let rel = detailed.iter().find(|s| s["name"] == "release").unwrap();
        assert_eq!(rel["description"], "Release checklist from manifest");
        assert_eq!(rel["enabled"], false);
        assert_eq!(rel["base_directory"], skill_dir.display().to_string());
        assert_eq!(rel["manifest"]["version"], "1.0.0");

        let pkg = prepare_github_skill_package(
            "https://github.com/acme/nola-skills/blob/main/skills/audit/SKILL.md",
            &dir.path().join(".nola/skills"),
        )
        .unwrap();
        assert_eq!(pkg["repo"], "acme/nola-skills");
        assert_eq!(pkg["name"], "audit");
        assert_eq!(
            pkg["raw_url"],
            "https://raw.githubusercontent.com/acme/nola-skills/main/skills/audit/SKILL.md"
        );
    }

    #[test]
    fn manifest_supporting_files_validate_traversal_limits_and_skill_skip_records_event() {
        // Valid supporting files list
        let ok = extract_manifest_supporting_files(&json!({
            "files": ["scripts/check.sh", "references/checklist.md"]
        }))
        .unwrap();
        assert_eq!(ok, vec!["scripts/check.sh", "references/checklist.md"]);

        // Reject traversal & absolute paths
        assert!(extract_manifest_supporting_files(&json!({
            "files": ["../escape.sh"]
        }))
        .is_err());
        assert!(extract_manifest_supporting_files(&json!({
            "files": ["/etc/passwd"]
        }))
        .is_err());

        // Reject > 16 files
        let too_many: Vec<String> = (0..17).map(|i| format!("file_{i}.txt")).collect();
        assert!(extract_manifest_supporting_files(&json!({ "files": too_many })).is_err());

        // SkillSkipTool persists skip event and module record
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
        let mut reg = ToolRegistry::default();
        register_tools(&suite, &mut reg);
        let ctx = ToolContext::new(dir.path()).with_session_id("sess-skip-1");
        let res = reg.execute(
            "skill_skip",
            &ctx,
            json!({"name": "tdd", "reason": "Pure documentation edit with no code changes"}),
        );
        assert!(!res.is_error);
        let evs = store.read_event_range("sess-skip-1", 1, 10).unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, "skill_skipped");
        assert_eq!(evs[0].payload["name"], "tdd");
    }
}
