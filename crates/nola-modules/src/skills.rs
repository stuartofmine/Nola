// SPDX-License-Identifier: Apache-2.0
//! Skills discovery (`SKILL.md` with YAML frontmatter) & tail-safe `skill_load` tool.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    CapabilityDomain, SkillDescriptor, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

const MAX_SKILL_DOWNLOAD_BYTES: u64 = 256 * 1024;

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
                if let Some(desc) = parse_skill_md(&skill_file, &raw) {
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

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(SkillLoadTool {
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
                let valid_seg = |s: &str| {
                    !s.is_empty()
                        && s != "."
                        && s != ".."
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                };
                if parts.len() != 2 || !valid_seg(parts[0]) || !valid_seg(parts[1]) {
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
                    || !skill_path.split('/').all(valid_seg)
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
                        ToolExecutionResult::ok_structured(
                            json!({"installed": true, "name": safe_name, "path": dest.display().to_string()}),
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
        let skills = discover_skills(&self.suite.workspace_root, self.suite.user_home.as_deref());
        let Some(name) = args.get("name").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
            let list: Vec<Value> = skills
                .iter()
                .map(|s| json!({"name": s.name, "description": s.description, "path": s.path}))
                .collect();
            let count = list.len();
            return ToolExecutionResult::ok_structured(
                json!({"count": count, "skills": list}),
                format!("Discovered {count} skill(s)"),
            );
        };
        match skills.into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(skill) => ToolExecutionResult::ok_structured(
                json!({
                    "name": skill.name,
                    "description": skill.description,
                    "path": skill.path,
                    "body": skill.body
                }),
                format!("Loaded skill '{}'", skill.name),
            ),
            None => ToolExecutionResult::err_structured(
                "skill_not_found",
                format!("skill '{name}' not found"),
                None,
            ),
        }
    }
}
