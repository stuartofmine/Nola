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
use std::path::{Path, PathBuf};

use globset::Glob;
use nola_store::{Store, StoreError};
use nola_tools::{canonicalize_nearest_ancestor, path_within_workspace};
use nola_types::{
    now_ms, sha256_hex, CapabilityDomain, PermissionAction, PermissionDecision, PermissionProfile,
    PermissionRule, RuleScope, ToolSpec,
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

/// Returns a concise reason if `path` targets a sensitive credential, key, or secret path.
pub fn is_sensitive_path(path: &str) -> Option<&'static str> {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized == "/etc/shadow" || normalized.ends_with("/etc/shadow") {
        return Some("/etc/shadow");
    }
    for comp in normalized.split('/').filter(|c| !c.is_empty()) {
        if comp == ".ssh" {
            return Some(".ssh");
        }
        if comp == ".env" || comp.starts_with(".env.") {
            return Some(".env");
        }
        if comp == ".aws" {
            return Some(".aws");
        }
        if comp == ".gnupg" {
            return Some(".gnupg");
        }
        if comp == "id_rsa" || comp.starts_with("id_rsa.") {
            return Some("id_rsa");
        }
        if comp == "id_ed25519" || comp.starts_with("id_ed25519.") {
            return Some("id_ed25519");
        }
        if comp.ends_with(".pem") {
            return Some(".pem");
        }
        if comp.ends_with(".key") {
            return Some(".key");
        }
        if comp == "credentials" || comp.contains("credentials") {
            return Some("credentials");
        }
        if comp == "secrets" || comp.contains("secrets") {
            return Some("secrets");
        }
        if comp.contains("mobile-access-token") {
            return Some("mobile-access-token");
        }
        if comp == "vault.enc" || comp.ends_with(".vault.enc") {
            return Some("vault.enc");
        }
        if comp.contains("keychain") {
            return Some("keychain");
        }
    }
    None
}

/// Returns true when a tool call performs an outbound external send/mutation over the network.
pub fn is_external_send(spec: &ToolSpec, args: &Value) -> bool {
    let tool_name = spec.schema.name.as_str();
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let method = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if tool_name == "messaging_manage" && matches!(action.as_str(), "send" | "create") {
        return true;
    }
    if spec.domain == CapabilityDomain::Network {
        if matches!(
            action.as_str(),
            "send" | "create" | "post" | "put" | "patch" | "delete" | "publish" | "upload" | "webhook"
        ) || matches!(method.as_str(), "post" | "put" | "patch" | "delete")
        {
            return true;
        }
        if !spec.read_only && action != "list" && action != "get" && action != "status" {
            return true;
        }
    }
    false
}

/// Durable session/workspace permission grant persisted in `Store` under `"permission_grant"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurablePermissionGrant {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub workspace_root: String,
    pub rule: PermissionRule,
    pub created_at_ms: i64,
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

impl DurablePermissionGrant {
    pub fn for_approval(
        session_id: Option<String>,
        workspace_root: &Path,
        tool_name: &str,
        resource_pattern: Option<String>,
    ) -> Self {
        let canonical_ws = std::fs::canonicalize(workspace_root)
            .unwrap_or_else(|_| workspace_root.to_path_buf())
            .display()
            .to_string();
        let scope_key = session_id.as_deref().unwrap_or("workspace");
        let res_key = resource_pattern.as_deref().unwrap_or("*");
        let raw_id = format!("{scope_key}|{canonical_ws}|{tool_name}|{res_key}");
        let id = format!("grant_{}", &sha256_hex(raw_id.as_bytes())[..16]);
        Self {
            id,
            session_id,
            workspace_root: canonical_ws,
            rule: PermissionRule {
                scope: RuleScope::User,
                action: PermissionAction::Allow,
                tool_pattern: tool_name.to_string(),
                resource_pattern,
            },
            created_at_ms: now_ms(),
            active: true,
        }
    }
}

/// Persist a durable permission grant in `Store` (`module_records` kind `"permission_grant"`).
pub fn persist_permission_grant(
    store: &Store,
    grant: &DurablePermissionGrant,
) -> Result<(), StoreError> {
    store.put_module_record("permission_grant", &grant.id, grant)
}

/// Load all active durable permission grants applicable to `(session_id, workspace_root)`.
pub fn load_active_permission_grants(
    store: &Store,
    session_id: &str,
    workspace_root: &Path,
) -> Vec<DurablePermissionGrant> {
    let canonical_ws = std::fs::canonicalize(workspace_root)
        .unwrap_or_else(|_| workspace_root.to_path_buf())
        .display()
        .to_string();
    store
        .list_module_records::<DurablePermissionGrant>("permission_grant")
        .unwrap_or_default()
        .into_iter()
        .filter(|g| {
            g.active
                && g.workspace_root == canonical_ws
                && (g.session_id.is_none() || g.session_id.as_deref() == Some(session_id))
        })
        .collect()
}

/// Extract a deterministic exact resource pattern for persisting a user approval grant.
pub fn primary_resource_pattern(tool_name: &str, args: &Value, workspace_root: &Path) -> Option<String> {
    let resources = extract_resources(tool_name, args, workspace_root);
    if resources.len() == 1 && resources[0] != tool_name {
        Some(escape_glob_literal(&resources[0]))
    } else {
        None
    }
}

fn escape_glob_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}') {
            out.push('[');
            out.push(ch);
            out.push(']');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Authoritative merged permission policy for a session (serializable for `--resume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub profile: PermissionProfile,
    pub workspace_trusted: bool,
    pub network_allowed: bool,
    pub rules: Vec<PermissionRule>,
    #[serde(default)]
    pub plan_mode: bool,
}

impl Default for EffectivePolicy {
    fn default() -> Self {
        Self {
            profile: PermissionProfile::Cautious,
            workspace_trusted: false,
            network_allowed: true,
            rules: Vec::new(),
            plan_mode: false,
        }
    }
}

impl EffectivePolicy {
    pub fn load(
        workspace_root: &Path,
        user_home: Option<&Path>,
        workspace_trusted: bool,
        cli_profile: Option<PermissionProfile>,
        network_allowed: bool,
    ) -> Self {
        let (policy, warnings) = Self::load_with_warnings(
            workspace_root,
            user_home,
            workspace_trusted,
            cli_profile,
            network_allowed,
        );
        for warning in warnings {
            eprintln!("nola-engine: policy config warning: {warning}");
        }
        policy
    }

    /// Load merged settings while collecting any malformed/unreadable config warnings without
    /// silently escalating permissions.
    pub fn load_with_warnings(
        workspace_root: &Path,
        user_home: Option<&Path>,
        workspace_trusted: bool,
        cli_profile: Option<PermissionProfile>,
        network_allowed: bool,
    ) -> (Self, Vec<String>) {
        let mut profile = PermissionProfile::Cautious;
        let mut rules: Vec<PermissionRule> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

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
                match read_settings_file(&path) {
                    Ok(Some(sf)) => {
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
                    Ok(None) => {}
                    Err(err) => warnings.push(err),
                }
            }
        }

        // 2. User settings (~/.nola/settings.json)
        if let Some(home) = user_home {
            match read_settings_file(&home.join(".nola/settings.json")) {
                Ok(Some(sf)) => {
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
                Ok(None) => {}
                Err(err) => warnings.push(err),
            }
        }

        // 3. CLI override wins on profile
        if let Some(p) = cli_profile {
            profile = p;
        }

        (
            Self {
                profile,
                workspace_trusted,
                network_allowed,
                rules,
                plan_mode: false,
            },
            warnings,
        )
    }

    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    pub fn with_plan_mode(mut self, enabled: bool) -> Self {
        self.plan_mode = enabled;
        self
    }

    /// Reload active durable session/workspace permission grants from `Store` into this policy.
    pub fn apply_durable_grants(&mut self, store: &Store, session_id: &str, workspace_root: &Path) {
        for grant in load_active_permission_grants(store, session_id, workspace_root) {
            if !self.rules.contains(&grant.rule) {
                self.rules.push(grant.rule);
            }
        }
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

        let is_git_read_only = tool_name == "git"
            && matches!(
                args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
                "status" | "diff" | "log" | "show" | "blame" | "worktree_list"
            );
        let is_git_worktree_write = tool_name == "git"
            && matches!(
                args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
                "worktree_create" | "worktree_remove"
            );
        let is_plan_mode_allowed = spec.read_only
            || is_git_read_only
            || matches!(
                tool_name,
                "propose_agent_plan"
                    | "update_agent_todos"
                    | "request_user_input"
                    | "render_ui_surface"
                    | "delete_ui_surface"
            )
            || (tool_name == "task_control"
                && matches!(
                    args.get("action").and_then(|v| v.as_str()).unwrap_or("list"),
                    "list" | "read" | "wait"
                ));

        // Plan Mode gate: blocks all mutating/write/exec tools until the plan is approved
        // or Plan Mode is disabled (`approve_agent_plan` / `set_plan_mode(false)`).
        if self.plan_mode && (!is_plan_mode_allowed || is_external_send(spec, args)) {
            return PermissionDecision {
                action: PermissionAction::Deny,
                reason: format!(
                    "Plan Mode is active: mutating/execution tool '{tool_name}' is blocked until the plan is approved (approve_agent_plan) or Plan Mode is disabled"
                ),
                domain,
            };
        }
        let resources = extract_resources(tool_name, args, workspace_root);
        let file_paths = extract_file_paths(tool_name, args, workspace_root);
        let sensitive_reason = file_paths.iter().find_map(|p| is_sensitive_path(p));
        let external_send = is_external_send(spec, args);
        let write_paths = if domain == CapabilityDomain::FilesWrite || is_git_worktree_write {
            extract_write_target_paths(tool_name, args, workspace_root)
        } else {
            Vec::new()
        };

        // Explicit rules: highest scope wins; within same scope Deny > Ask > Allow!
        // For Allow rules with a resource_pattern, EVERY extracted resource MUST match
        // (so multi-target tools like move_file / read_many_files cannot smuggle an unallowed path).
        // For Ask/Deny rules, ANY matching resource triggers the restriction.
        let mut matched: Option<&PermissionRule> = None;
        for rule in &self.rules {
            if !tool_matches(&rule.tool_pattern, tool_name, domain) {
                continue;
            }
            if let Some(rp) = &rule.resource_pattern {
                let res_matches = match rule.action {
                    PermissionAction::Allow => {
                        !resources.is_empty() && resources.iter().all(|res| glob_matches(rp, res))
                    }
                    PermissionAction::Ask | PermissionAction::Deny => {
                        resources.iter().any(|res| glob_matches(rp, res))
                    }
                };
                if !res_matches {
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

        // Security invariant: for FilesWrite & git worktree mutations, enforce workspace boundary
        // BEFORE Project/ProjectLocal Allow rules! Only AllowAll profile or an explicit
        // User/CliOverride Allow rule can permit writing outside the workspace.
        if domain == CapabilityDomain::FilesWrite || is_git_worktree_write {
            if write_paths.is_empty() && tool_name != "workspace_checkpoint" {
                return PermissionDecision {
                    action: PermissionAction::Deny,
                    reason: format!("unable to determine target path(s) for write tool '{tool_name}'"),
                    domain,
                };
            }
            for target in &write_paths {
                if !path_within_workspace(workspace_root, target) {
                    let explicit_user_override = matched.is_some_and(|r| {
                        r.action == PermissionAction::Allow
                            && matches!(r.scope, RuleScope::User | RuleScope::CliOverride)
                    });
                    let explicit_deny = matched.is_some_and(|r| r.action == PermissionAction::Deny);
                    if explicit_deny {
                        break;
                    }
                    if !explicit_user_override && self.profile != PermissionProfile::AllowAll {
                        return PermissionDecision {
                            action: PermissionAction::Deny,
                            reason: format!(
                                "write target '{}' escapes workspace boundary",
                                target.display()
                            ),
                            domain,
                        };
                    }
                }
            }
        }

        if let Some(rule) = matched {
            let project_allow_blocked_by_guard = rule.action == PermissionAction::Allow
                && !matches!(rule.scope, RuleScope::User | RuleScope::CliOverride)
                && self.profile != PermissionProfile::AllowAll
                && (sensitive_reason.is_some() || external_send);
            if !project_allow_blocked_by_guard {
                return PermissionDecision {
                    action: rule.action,
                    reason: format!("matched {:?} rule for '{}'", rule.scope, rule.tool_pattern),
                    domain,
                };
            }
        }

        // Sensitive-path guard: even read-only tools MUST ask before touching credentials/keys/secrets
        // unless running under AllowAll (or explicitly allowed by a User/CliOverride rule above).
        if let Some(reason) = sensitive_reason {
            if self.profile != PermissionProfile::AllowAll {
                let msg = if spec.read_only || is_git_read_only || domain == CapabilityDomain::FilesRead
                {
                    format!("sensitive file read requires confirmation: {reason}")
                } else {
                    format!("sensitive file access requires confirmation: {reason}")
                };
                return PermissionDecision {
                    action: PermissionAction::Ask,
                    reason: msg,
                    domain,
                };
            }
        }

        // Outbound external-send guard: NEVER auto-allow outbound send/POST under Cautious/AcceptEdits.
        if external_send && self.profile != PermissionProfile::AllowAll {
            return PermissionDecision {
                action: PermissionAction::Ask,
                reason: format!("outbound external send via '{tool_name}' requires confirmation"),
                domain,
            };
        }

        // Default profile behavior
        if spec.read_only
            || is_git_read_only
            || matches!(
                tool_name,
                "update_agent_todos"
                    | "propose_agent_plan"
                    | "request_user_input"
                    | "render_ui_surface"
                    | "delete_ui_surface"
            )
            || (tool_name == "task_control"
                && matches!(
                    args.get("action").and_then(|v| v.as_str()).unwrap_or("list"),
                    "list" | "read" | "wait"
                ))
        {
            return PermissionDecision {
                action: PermissionAction::Allow,
                reason: "read-only or internal coordination tool allowed".into(),
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

fn read_settings_file(path: &Path) -> Result<Option<SettingsFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "failed to read settings file '{}': {e}",
                path.display()
            ));
        }
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("malformed settings JSON in '{}': {e}", path.display()))
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

fn resolve_canonical_path(workspace_root: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    canonicalize_nearest_ancestor(&joined)
}

fn unquote_diff_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

fn tokenize_shell_command(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' && q == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                c if c.is_whitespace()
                    || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')' | '`') =>
                {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                other => current.push(other),
            },
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut expanded = Vec::with_capacity(tokens.len());
    for tok in tokens {
        let cleaned = tok.trim_matches(|c| matches!(c, '\'' | '"'));
        if !cleaned.is_empty() {
            expanded.push(cleaned.to_string());
            if let Some((_, rhs)) = cleaned.split_once('=') {
                let rhs_clean = rhs.trim_matches(|c| matches!(c, '\'' | '"'));
                if !rhs_clean.is_empty() {
                    expanded.push(rhs_clean.to_string());
                }
            }
        }
    }
    expanded
}

fn extract_patch_target_paths(patch: &str, workspace_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let rest = if let Some(r) = line.strip_prefix("+++ ") {
            r
        } else if let Some(r) = line.strip_prefix("--- ") {
            r
        } else {
            continue;
        };
        let raw_tab = rest.split('\t').next().unwrap_or(rest).trim();
        let unquoted = unquote_diff_path(raw_tab);
        if unquoted.is_empty() || unquoted == "/dev/null" {
            continue;
        }
        let cleaned = unquoted
            .strip_prefix("a/")
            .or_else(|| unquoted.strip_prefix("b/"))
            .unwrap_or(&unquoted);
        if !cleaned.is_empty() {
            paths.push(resolve_canonical_path(workspace_root, cleaned));
        }
    }
    paths
}

fn extract_write_target_paths(tool_name: &str, args: &Value, workspace_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if tool_name == "apply_patch" {
        if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
            out.extend(extract_patch_target_paths(patch, workspace_root));
        }
        return out;
    }
    for key in ["path", "source", "destination", "target"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
            if !p.trim().is_empty() {
                out.push(resolve_canonical_path(workspace_root, p));
            }
        }
    }
    if let Some(arr) = args.get("paths").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(p) = item.as_str() {
                if !p.trim().is_empty() {
                    out.push(resolve_canonical_path(workspace_root, p));
                }
            }
        }
    }
    out
}

fn extract_file_paths(tool_name: &str, args: &Value, workspace_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if tool_name == "apply_patch" {
        if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
            for line in patch.lines() {
                let rest = if let Some(r) = line.strip_prefix("+++ ") {
                    r
                } else if let Some(r) = line.strip_prefix("--- ") {
                    r
                } else {
                    continue;
                };
                let raw_tab = rest.split('\t').next().unwrap_or(rest).trim();
                let unquoted = unquote_diff_path(raw_tab);
                if unquoted.is_empty() || unquoted == "/dev/null" {
                    continue;
                }
                let cleaned = unquoted
                    .strip_prefix("a/")
                    .or_else(|| unquoted.strip_prefix("b/"))
                    .unwrap_or(&unquoted);
                if !cleaned.is_empty() {
                    out.push(cleaned.to_string());
                    out.push(
                        resolve_canonical_path(workspace_root, cleaned)
                            .display()
                            .to_string(),
                    );
                }
            }
        }
    }
    if matches!(tool_name, "bash" | "pty_exec") {
        if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
            for tok in tokenize_shell_command(cmd) {
                out.push(tok);
            }
        }
    }
    if tool_name == "glob" {
        if let Some(pat) = args.get("pattern").and_then(|v| v.as_str()) {
            let trimmed = pat.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
    if tool_name == "grep" {
        if let Some(inc) = args.get("include").and_then(|v| v.as_str()) {
            let trimmed = inc.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
    for key in ["path", "source", "destination", "target", "cwd", "worktree_path"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
                out.push(
                    resolve_canonical_path(workspace_root, trimmed)
                        .display()
                        .to_string(),
                );
            }
        }
    }
    if let Some(arr) = args.get("paths").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(p) = item.as_str() {
                let trimmed = p.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                    out.push(
                        resolve_canonical_path(workspace_root, trimmed)
                            .display()
                            .to_string(),
                    );
                }
            }
        }
    }
    out
}

fn extract_resources(tool_name: &str, args: &Value, workspace_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
        out.push(cmd.trim().to_string());
    }
    for key in ["url", "endpoint"] {
        if let Some(u) = args.get(key).and_then(|v| v.as_str()) {
            let trimmed = u.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
    if tool_name == "apply_patch" {
        if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
            for p in extract_patch_target_paths(patch, workspace_root) {
                out.push(p.display().to_string());
            }
        }
    }
    for key in ["path", "source", "destination", "target", "cwd", "worktree_path"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
            if !p.trim().is_empty() {
                out.push(resolve_canonical_path(workspace_root, p).display().to_string());
            }
        }
    }
    if let Some(arr) = args.get("paths").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(p) = item.as_str() {
                if !p.trim().is_empty() {
                    out.push(resolve_canonical_path(workspace_root, p).display().to_string());
                }
            }
        }
    }
    if out.is_empty() {
        out.push(tool_name.to_string());
    }
    out
}
