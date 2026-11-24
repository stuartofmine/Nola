// SPDX-License-Identifier: Apache-2.0
//! Unified `EffectivePolicy` merging Defaults -> Trusted Project -> User -> CLI Overrides.
//!
//! Eliminates v1's 4-way policy drift:
//! - `bash` and `pty_exec` share `CapabilityDomain::Process` rules.
//! - Symlink-safe workspace boundary check via `nola_tools::path_within_workspace`.
//! - Untrusted workspaces NEVER load `.nola/settings.json` or `.nola/settings.local.json`.
//! - User/CLI `Deny` ALWAYS overrides Project `Allow`.
//! - Fully serializable so `--resume` preserves exact permission profile & rules.

use std::fs;
use std::path::Path;

use globset::Glob;
use nola_tools::path_within_workspace;
use nola_types::{
    CapabilityDomain, PermissionAction, PermissionDecision, PermissionProfile, PermissionRule,
    RuleScope, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    permission_profile: Option<PermissionProfile>,
    #[serde(default)]
    rules: Vec<SettingsRule>,
}

#[derive(Debug, Deserialize)]
struct SettingsRule {
    action: PermissionAction,
    tool_pattern: String,
    #[serde(default)]
    resource_pattern: Option<String>,
}

/// Authoritative merged permission policy for a session (serializable for `--resume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub profile: PermissionProfile,
    pub workspace_trusted: bool,
    pub network_allowed: bool,
    pub rules: Vec<PermissionRule>,
}

impl EffectivePolicy {
    pub fn load(
        workspace_root: &Path,
        user_home: Option<&Path>,
        workspace_trusted: bool,
        cli_profile: Option<PermissionProfile>,
        network_allowed: bool,
    ) -> Self {
        let mut profile = PermissionProfile::Cautious;
        let mut rules: Vec<PermissionRule> = Vec::new();

        // 1. Project settings (ONLY if workspace is explicitly trusted!)
        if workspace_trusted {
            let proj_files = [
                (workspace_root.join(".nola/settings.json"), RuleScope::Project),
                (
                    workspace_root.join(".nola/settings.local.json"),
                    RuleScope::ProjectLocal,
                ),
            ];
            for (path, scope) in proj_files {
                if let Some(sf) = read_settings_file(&path) {
                    if let Some(p) = sf.permission_profile {
                        profile = p;
                    }
                    for r in sf.rules {
                        rules.push(PermissionRule {
                            scope,
                            action: r.action,
                            tool_pattern: r.tool_pattern,
                            resource_pattern: r.resource_pattern,
                        });
                    }
                }
            }
        }

        // 2. User settings (~/.nola/settings.json)
        if let Some(home) = user_home {
            if let Some(sf) = read_settings_file(&home.join(".nola/settings.json")) {
                if let Some(p) = sf.permission_profile {
                    profile = p;
                }
                for r in sf.rules {
                    rules.push(PermissionRule {
                        scope: RuleScope::User,
                        action: r.action,
                        tool_pattern: r.tool_pattern,
                        resource_pattern: r.resource_pattern,
                    });
                }
            }
        }

        // 3. CLI override wins on profile
        if let Some(p) = cli_profile {
            profile = p;
        }

        Self {
            profile,
            workspace_trusted,
            network_allowed,
            rules,
        }
    }

    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    /// Evaluate whether a tool call is Allowed, Ask, or Denied.
    pub fn evaluate(
        &self,
        spec: &ToolSpec,
        args: &Value,
        workspace_root: &Path,
    ) -> PermissionDecision {
        let tool_name = spec.schema.name.as_str();
        let domain = spec.domain;

        // Hard network policy check
        if domain == CapabilityDomain::Network && !self.network_allowed {
            return PermissionDecision {
                action: PermissionAction::Deny,
                reason: "network policy forbids outbound network tools".into(),
                domain,
            };
        }

        let resource = extract_primary_resource(tool_name, args, workspace_root);

        // Explicit rules: highest scope wins; within same scope Deny > Ask > Allow!
        let mut matched: Option<&PermissionRule> = None;
        for rule in &self.rules {
            if !tool_matches(&rule.tool_pattern, tool_name, domain) {
                continue;
            }
            if let Some(rp) = &rule.resource_pattern {
                if !resource
                    .as_deref()
                    .map(|res| glob_matches(rp, res))
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            matched = match matched {
                None => Some(rule),
                Some(prev) => {
                    if rule.scope > prev.scope
                        || (rule.scope == prev.scope
                            && action_strictness(rule.action) > action_strictness(prev.action))
                    {
                        Some(rule)
                    } else {
                        Some(prev)
                    }
                }
            };
        }

        if let Some(rule) = matched {
            return PermissionDecision {
                action: rule.action,
                reason: format!("matched {:?} rule for '{}'", rule.scope, rule.tool_pattern),
                domain,
            };
        }

        // Check symlink-safe workspace boundary for file writes
        if domain == CapabilityDomain::FilesWrite {
            if let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) {
                let target = if Path::new(raw_path).is_absolute() {
                    Path::new(raw_path).to_path_buf()
                } else {
                    workspace_root.join(raw_path)
                };
                if !path_within_workspace(workspace_root, &target) {
                    return match self.profile {
                        PermissionProfile::AllowAll => PermissionDecision {
                            action: PermissionAction::Allow,
                            reason: "allow_all profile permits external write".into(),
                            domain,
                        },
                        _ => PermissionDecision {
                            action: PermissionAction::Deny,
                            reason: format!(
                                "write target '{}' escapes workspace boundary",
                                target.display()
                            ),
                            domain,
                        },
                    };
                }
            }
        }

        // Default profile behavior
        if spec.read_only {
            return PermissionDecision {
                action: PermissionAction::Allow,
                reason: "read-only tool allowed".into(),
                domain,
            };
        }

        match self.profile {
            PermissionProfile::AllowAll => PermissionDecision {
                action: PermissionAction::Allow,
                reason: "allow_all profile".into(),
                domain,
            },
            PermissionProfile::AcceptEdits => {
                if domain == CapabilityDomain::FilesWrite {
                    PermissionDecision {
                        action: PermissionAction::Allow,
                        reason: "accept_edits allows workspace file mutations".into(),
                        domain,
                    }
                } else {
                    PermissionDecision {
                        action: PermissionAction::Ask,
                        reason: format!("accept_edits requires approval for {domain:?}"),
                        domain,
                    }
                }
            }
            PermissionProfile::Cautious => PermissionDecision {
                action: PermissionAction::Ask,
                reason: format!("cautious profile requires approval for {tool_name}"),
                domain,
            },
        }
    }
}

fn read_settings_file(path: &Path) -> Option<SettingsFile> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn action_strictness(a: PermissionAction) -> u8 {
    match a {
        PermissionAction::Allow => 1,
        PermissionAction::Ask => 2,
        PermissionAction::Deny => 3,
    }
}

fn tool_matches(pattern: &str, tool_name: &str, domain: CapabilityDomain) -> bool {
    let p = pattern.trim();
    if p == "*" || p == tool_name {
        return true;
    }
    // Treat `bash` and `pty_exec` identically when matching process patterns!
    if (p == "bash" || p == "pty_exec" || p == "process") && domain == CapabilityDomain::Process {
        return true;
    }
    let domain_str = format!("{domain:?}").to_ascii_lowercase();
    if p.to_ascii_lowercase() == domain_str {
        return true;
    }
    glob_matches(p, tool_name)
}

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    Glob::new(pattern)
        .map(|g| g.compile_matcher().is_match(candidate))
        .unwrap_or_else(|_| pattern == candidate)
}

fn extract_primary_resource(tool_name: &str, args: &Value, workspace_root: &Path) -> Option<String> {
    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
        return Some(cmd.trim().to_string());
    }
    if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        return Some(url.trim().to_string());
    }
    if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
        let full = if Path::new(p).is_absolute() {
            Path::new(p).to_path_buf()
        } else {
            workspace_root.join(p)
        };
        return Some(full.display().to_string());
    }
    Some(tool_name.to_string())
}
