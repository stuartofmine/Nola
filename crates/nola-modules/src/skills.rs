// SPDX-License-Identifier: Apache-2.0
//! Skills discovery (`SKILL.md` with YAML frontmatter) & tail-safe `skill_load` tool.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    CapabilityDomain, SkillDescriptor, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

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

    let mut name = fallback_name;
    let mut description = String::new();
    let mut body = raw.trim().to_string();

    if let Some(rest) = raw.strip_prefix("---\n") {
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
