use chrono::{Local, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io;
use std::io::{BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};

use base64::Engine as _;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::application_port::CliApplicationPort;
use crate::config_paths::expand_home;
use crate::i18n::{
    browser_accept_language, localize_tool_error_display, tr, Message,
};
use crate::runtime_interaction::{
    prompt_permission,
    permission_summary, permission_interaction_available,
    PermissionChoice, PermissionPrompt, PermissionPromptKind,
};
use crate::runtime_control::{current_execution_control, RuntimeExecutionControl};
use crate::permissions::{
    evaluate_tool_request, has_explicit_ask_rule,
    PermissionDecision,
};
use crate::skill_runtime::SkillRuntimeContext;
use crate::store::{DbBackgroundTask, DbWorkspaceCheckpoint, NolaStore};

const MAX_READ_BYTES: u64 = 1_000_000;
const MAX_WRITE_BYTES: usize = 500_000;
const MAX_EDITED_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PATCH_BYTES: usize = 1_000_000;
const MAX_DIFF_CHARS: usize = 40_000;
const MAX_BACKGROUND_OUTPUT_BYTES: i64 = 100_000;
const MAX_TOOL_RESULT_CHARS: usize = 4_000;
const MAX_WEB_SEARCH_RESULT_CHARS: usize = 8_000;
const MAX_TOOL_OUTPUT_LIST: i64 = 100;
const MAX_TOOL_OUTPUT_READ_REQUEST_BYTES: i64 = 200_000;
const MAX_TOOL_OUTPUT_CONTEXT_LINES: i64 = 5;
const MAX_TOOL_OUTPUT_MATCHES: i64 = 200;
// Keep read-back slices below MAX_TOOL_RESULT_CHARS. Otherwise the generic
// result clamp persists the slice as a new output and invites the model to
// recursively read the newly-created file instead of advancing the offset.
const MAX_TOOL_OUTPUT_SLICE_BYTES: i64 = 2_500;
const MAX_MULTI_EDITS: usize = 50;
const MAX_TOOL_SEARCH_RESULTS: i64 = 20;
const MAX_READ_MANY_FILES: usize = 20;
const MAX_READ_MANY_BYTES: i64 = 400_000;
const LARGE_FILE_PREVIEW_THRESHOLD_BYTES: u64 = 50_000;
const LARGE_FILE_AUTO_PREVIEW_BYTES: i64 = 16_000;
const MAX_NOTEBOOK_CELLS: i64 = 500;
const MAX_NOTEBOOK_SOURCE_CHARS: usize = 200_000;
const MAX_NOTEBOOK_OUTPUT_CHARS: i64 = 40_000;
const MAX_OUTLINE_SYMBOLS: i64 = 1_000;
const MAX_REPO_MAP_FILES: i64 = 500;
const MAX_REPO_MAP_SYMBOLS_PER_FILE: i64 = 100;
const MAX_CODE_SEARCH_MATCHES: i64 = 2_000;
const MAX_WORKSPACE_SYMBOL_MATCHES: i64 = 2_000;
const MAX_SYMBOL_SEARCH_MATCHES: i64 = 2_000;
const MAX_IMPLEMENTATION_MATCHES: i64 = 1_000;
const MAX_CODE_NAVIGATION_MATCHES: i64 = 500;
const MAX_CODE_DIAGNOSTICS: i64 = 500;
const MAX_DEPENDENCY_IMPORTS: i64 = 500;
const MAX_CONVENTION_FILES: i64 = 300;
const MAX_PROJECT_SCAN_FILES: i64 = 20_000;
const MAX_RELATED_RESULTS: i64 = 500;
const MAX_REPLACE_FILES: i64 = 500;
const MAX_INSTRUCTION_BYTES: i64 = 100_000;
const MAX_SKILL_BYTES: i64 = 100_000;
const MAX_SKILL_FILES: i64 = 30;
const MAX_AGENT_BYTES: i64 = 100_000;
const MAX_COMMAND_BYTES: i64 = 100_000;
const MAX_AGENT_CONFIG_BYTES: i64 = 100_000;
const MAX_CHECKPOINTS: usize = 20;
const MAX_ARTIFACT_SCAN_FILES: usize = 20_000;
const MAX_PLAN_STEPS: usize = 12;
const MAX_PLAN_CHARS: usize = 10_000;

/// Builds the model-visible Skill index from the exact roots frozen by the
/// embedding Host. The index contains metadata and provenance only; full
/// instructions are loaded later by the `Skill` tool.
pub(crate) fn configured_skill_context_index(skill_dirs: &[PathBuf]) -> Result<Vec<Value>, String> {
    Ok(configured_skill_context_entries(skill_dirs)?
        .into_iter()
        .map(|(_, metadata)| metadata)
        .collect())
}

/// Retains the exact filesystem identity for execution. Display metadata may
/// encode paths lossily and must never be decoded back into execution authority.
pub(crate) fn configured_skill_context_entries(
    skill_dirs: &[PathBuf],
) -> Result<Vec<(PathBuf, Value)>, String> {
    let mut by_id = BTreeMap::<String, (PathBuf, Value)>::new();
    for root in skill_dirs {
        let metadata = fs::metadata(root).map_err(|error| {
            format!(
                "Unable to inspect configured Skill root {}: {error}",
                root.display()
            )
        })?;
        if !metadata.is_dir() {
            return Err(format!(
                "Configured Skill root is not a directory: {}",
                root.display()
            ));
        }
        let canonical_root = root.canonicalize().map_err(|error| {
            format!(
                "Unable to resolve configured Skill root {}: {error}",
                root.display()
            )
        })?;
        let paths = discover_configured_skill_files(&canonical_root)?;
        for path in paths {
            let canonical_path = path.canonicalize().map_err(|error| {
                format!(
                    "Unable to resolve configured Skill {}: {error}",
                    path.display()
                )
            })?;
            let directory = canonical_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("Configured Skill has no parent: {}", path.display()))?;
            if directory != canonical_root && !directory.starts_with(&canonical_root) {
                return Err(format!(
                    "Configured Skill resolved outside its authorized root: {}",
                    canonical_path.display()
                ));
            }
            let content = read_configured_skill_index_file(&canonical_path)
                .map_err(|error| format!("Unable to read {}: {error}", canonical_path.display()))?;
            let id = skill_title(&canonical_path, &content).trim().to_string();
            if id.is_empty() {
                continue;
            }
            if let Some((existing, _)) = by_id.get(&id) {
                if existing != &directory {
                    return Err(format!(
                        "Ambiguous configured Skill id {id:?}: {} and {}",
                        existing.display(),
                        directory.display()
                    ));
                }
                continue;
            }
            by_id.insert(
                id.clone(),
                (
                    directory.clone(),
                    json!({
                        "id": id,
                        "name": skill_title(&canonical_path, &content),
                        "description": skill_description(&content),
                        "enabled": true,
                        "source": "host_configured",
                        "base_directory": directory.to_string_lossy(),
                    }),
                ),
            );
        }
    }
    Ok(by_id.into_values().collect())
}

/// A configured directory is a complete catalog, not a best-effort list.
/// Exceeding the existing discovery budget is explicit rather than returning a
/// partial set of canonical ids. One extra candidate detects overflow without
/// reading its body. No directory is traversed before containment
/// is checked, including directories reached through symbolic links.
fn discover_configured_skill_files(canonical_root: &Path) -> Result<Vec<PathBuf>, String> {
    const DISCOVERY_LIMIT: usize = 100;
    let mut stack = vec![canonical_root.to_path_buf()];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    while let Some(directory) = stack.pop() {
        let canonical_directory = directory.canonicalize().map_err(|error| {
            format!(
                "Unable to resolve configured Skill directory {}: {error}",
                directory.display()
            )
        })?;
        if !canonical_directory.starts_with(canonical_root) {
            return Err(format!(
                "Configured Skill directory resolved outside its authorized root: {}",
                canonical_directory.display()
            ));
        }
        if !visited.insert(canonical_directory.clone()) {
            continue;
        }
        let entries = fs::read_dir(&canonical_directory).map_err(|error| {
            format!(
                "Unable to read configured Skill directory {}: {error}",
                canonical_directory.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "Unable to enumerate configured Skill directory {}: {error}",
                    canonical_directory.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::metadata(&path).map_err(|error| {
                format!(
                    "Unable to inspect configured Skill entry {}: {error}",
                    path.display()
                )
            })?;
            let name = entry.file_name();
            if metadata.is_dir() {
                if !should_skip_dir(&name.to_string_lossy()) {
                    stack.push(path);
                }
            } else if metadata.is_file() && name == "SKILL.md" {
                files.push(path);
                if files.len() > DISCOVERY_LIMIT {
                    return Err(format!(
                        "Configured Skill discovery limit exceeded ({DISCOVERY_LIMIT}) in {}",
                        canonical_root.display()
                    ));
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn read_configured_skill_index_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    // The generic prefix reader treats a metadata failure as length zero.
    // Catalog admission must preserve this error instead of advertising ids.
    let metadata = file.metadata()?;
    let limit_error = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Configured Skill exceeds the {MAX_SKILL_BYTES}-byte instruction limit"),
        )
    };
    if metadata.len() > MAX_SKILL_BYTES as u64 {
        return Err(limit_error());
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_SKILL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SKILL_BYTES as usize {
        return Err(limit_error());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn optional_custom_skill_context_index(root: &Path) -> Result<Vec<Value>, String> {
    Ok(optional_custom_skill_context_entries(root)?
        .into_iter()
        .map(|(_, metadata)| metadata)
        .collect())
}

pub(crate) fn optional_custom_skill_context_entries(
    root: &Path,
) -> Result<Vec<(PathBuf, Value)>, String> {
    let exists = root.try_exists().map_err(|error| {
        format!(
            "Unable to inspect optional custom Skill root {}: {error}",
            root.display()
        )
    })?;
    if !exists {
        // A dangling link is not an absent optional directory. Do not hide an
        // existing but invalid path under the default-managed selection.
        match fs::symlink_metadata(root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "Unable to inspect optional custom Skill root {}: {error}",
                    root.display()
                ));
            }
            Ok(_) => {
                return Err(format!(
                    "Optional custom Skill root exists but cannot be resolved: {}",
                    root.display()
                ));
            }
        }
    }
    configured_skill_context_entries(&[root.to_path_buf()])
}

pub(crate) fn configured_skill_catalog_prompt(skill_dirs: &[PathBuf]) -> Result<String, String> {
    Ok(skill_catalog_prompt_from_items(
        &configured_skill_context_index(skill_dirs)?,
    ))
}

impl SkillRuntimeContext {
    pub(crate) fn index(&self) -> Result<Vec<Value>, String> {
        let mut items = match self.configured_dirs() {
            Some(skill_dirs) => configured_skill_context_index(skill_dirs),
            None => default_cli_skill_context_index(),
        }?;
        for skill in self.plugin_skills()? {
            let path = skill.directory.join("SKILL.md");
            let content = read_configured_skill_index_file(&path).map_err(|_| "plugin Skill unavailable")?;
            if items.iter().any(|item| item["id"].as_str() == Some(&skill.resource_id)) {
                return Err("plugin Skill identity conflicts with configured catalog".into());
            }
            items.push(json!({"id":skill.resource_id,"name":skill_title(&path, &content),
                "description":skill_description(&content),"enabled":true,"source":"plugin",
                "base_directory":skill.directory.to_string_lossy()}));
        }
        Ok(items)
    }

    pub(crate) fn invocation_schema(&self) -> Value {
        // The catalog schema is presentation, not execution authority. Runtime
        // composition validates index errors before exposing a directory, and
        // invocation independently resolves the exact id against these roots.
        let ids = self
            .index()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string));
        skill_invocation_schema_from_ids(ids)
    }

    pub(crate) fn catalog_prompt(&self) -> Result<String, String> {
        Ok(skill_catalog_prompt_from_items(&self.index()?))
    }
}

fn default_cli_skill_context_index() -> Result<Vec<Value>, String> {
    let mut items = crate::builtin_skills::context_index(&[], &[])?;
    let custom_root = crate::builtin_skills::managed_custom_root()?;
    let custom = optional_custom_skill_context_index(&custom_root)?;
    let mut ids = items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for item in custom {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if ids.insert(id.to_string()) {
            items.push(item);
        }
    }
    Ok(items)
}

#[cfg(test)]
include!("contracts_execution_tests.rs");

pub(crate) fn skill_catalog_prompt_from_items(items: &[Value]) -> String {
    const DESCRIPTION_LIMIT: usize = 250;
    const CATALOG_LIMIT: usize = 8_000;
    let mut lines = vec![
        "NOLA_SKILL_CATALOG_V1".to_string(),
        "Each entry is `- <canonical skill_id>: <description>`. Pass the exact canonical skill_id to Skill.skill; display names and titles are not valid ids.".to_string(),
    ];
    let mut used: usize = lines.iter().map(String::len).sum();
    for item in items {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(DESCRIPTION_LIMIT)
            .collect::<String>();
        let line = format!("- {id}: {description}");
        if used.saturating_add(line.len()) > CATALOG_LIMIT {
            break;
        }
        used = used.saturating_add(line.len());
        lines.push(line);
    }
    lines.join("\n")
}

/// Returns whether the current Agent Host is allowed to expose and execute a
/// capability. The manifest is supplied by the embedding client at process
/// launch and is intentionally independent from user wording. Standalone CLI
/// and Web runs without a manifest retain their ordinary capability surface.
pub(crate) fn host_capability_enabled(ctx: &ToolContext, name: &str) -> bool {
    // Execution checks always recapture current availability. A directory
    // snapshot is not a cached grant or frozen execution authority.
    HostCapabilityAvailability::capture(&ctx.skills).enabled(name)
}

/// One directory build uses one captured set of availability facts. These
/// flags neither authorize a tool call nor own its configured Skill roots.
pub(crate) struct HostCapabilityAvailability {
    manifest_valid: bool,
    skills_enabled: bool,
    memory_enabled: bool,
    native_computer_enabled: bool,
    native_browser_enabled: bool,
    managed_browser_enabled: bool,
}

fn verified_artifact_mime(path: &Path) -> Option<String> {
    use std::io::Read;
    let declared = mime_guess::from_path(path).first_raw()?;
    if !declared.starts_with("image/") && declared != "application/pdf" {
        return Some(declared.into());
    }
    let mut header = [0u8; 512];
    let length = std::fs::File::open(path).ok()?.read(&mut header).ok()?;
    let bytes = &header[..length];
    let detected = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { "image/png" }
        else if bytes.starts_with(b"\xff\xd8\xff") { "image/jpeg" }
        else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { "image/gif" }
        else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { "image/webp" }
        else if bytes.starts_with(b"%PDF-") { "application/pdf" }
        else { "application/octet-stream" };
    Some(detected.into())
}

impl HostCapabilityAvailability {
    pub(crate) fn capture(skills: &SkillRuntimeContext) -> Self {
        let manifest = std::env::var("NOLA_HOST_CAPABILITY_MANIFEST").ok();
        Self::from_captured(skills.enabled(), manifest.as_deref())
    }

    fn from_captured(configured_skills_enabled: bool, raw_manifest: Option<&str>) -> Self {
        let manifest = raw_manifest
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .filter(|value| value.get("schema_version").and_then(Value::as_u64) == Some(1))
            .filter(|value| ["skills_enabled", "memory_enabled", "native_computer_enabled",
                "native_browser_enabled", "managed_browser_enabled"].iter()
                .all(|field| value.get(field).is_none_or(Value::is_boolean)))
            .filter(|value| value.get("enabled_skill_ids").is_none_or(|ids|
                ids.as_array().is_some_and(|ids| ids.iter().all(Value::is_string))));
        let manifest_valid = raw_manifest.is_none() || manifest.is_some();
        let enabled = |field: &str| {
            manifest
                .as_ref()
                .and_then(|value| value.get(field))
                .and_then(Value::as_bool)
                .unwrap_or(true)
        };
        Self {
            manifest_valid,
            skills_enabled: configured_skills_enabled && enabled("skills_enabled"),
            memory_enabled: enabled("memory_enabled"),
            native_computer_enabled: enabled("native_computer_enabled"),
            native_browser_enabled: enabled("native_browser_enabled"),
            managed_browser_enabled: enabled("managed_browser_enabled"),
        }
    }

    pub(crate) fn enabled(&self, name: &str) -> bool {
        if !self.manifest_valid { return false; }
        match name {
            "language_service_query" | "terminal_input" => cfg!(unix),
            "Skill" | "skill_invoke" | "skill_update" | "skill_skip" => self.skills_enabled,
            "memory_search" | "memory_save" => self.memory_enabled,
            name if name.starts_with("computer_") => self.native_computer_enabled,
            name if name.starts_with("chrome_native_") => self.native_browser_enabled,
            name if name.starts_with("browser_") => {
                self.managed_browser_enabled || self.native_browser_enabled
            }
            _ => true,
        }
    }
}

#[test]
fn invalid_host_manifest_never_enables_default_capabilities() {
    for raw in ["", "not-json", "{}", "null", r#"{"schema_version":2}"#,
        r#"{"schema_version":1,"memory_enabled":"false"}"#,
        r#"{"schema_version":1,"enabled_skill_ids":[42]}"#] {
        let captured = HostCapabilityAvailability::from_captured(true, Some(raw));
        assert!(!captured.manifest_valid, "{raw}");
        for tool in ["read_file", "memory_save", "terminal_input", "language_service_query", "Skill"] {
            assert!(!captured.enabled(tool), "{raw}: {tool}");
        }
    }
    assert!(HostCapabilityAvailability::from_captured(true, None).enabled("read_file"));
    let valid = HostCapabilityAvailability::from_captured(true,
        Some(r#"{"schema_version":1,"memory_enabled":false}"#));
    assert!(valid.manifest_valid && valid.enabled("read_file"));
    assert!(!valid.enabled("memory_save"));
}

pub(crate) fn host_skill_enabled(ctx: &ToolContext, skill_id: &str) -> bool {
    if !host_capability_enabled(ctx, "Skill") {
        return false;
    }
    let Ok(raw_manifest) = std::env::var("NOLA_HOST_CAPABILITY_MANIFEST") else {
        return true;
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&raw_manifest) else {
        return false;
    };
    let Some(enabled_ids) = manifest.get("enabled_skill_ids") else {
        return true;
    };
    enabled_ids
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(skill_id)))
}
const MAX_LINE_COUNT: i64 = 2_000;
const MAX_WEB_DOWNLOAD_BYTES: u64 = 2_000_000;
const MAX_WEB_RESPONSE_CHARS: i64 = 80_000;
const MAX_NATIVE_BROWSER_SCREENSHOT_BYTES: usize = 12 * 1024 * 1024;
const MIN_SCREEN_COORDINATE: f64 = -100_000.0;
const MAX_SCREEN_COORDINATE: f64 = 100_000.0;
const MIN_ELEMENT_OFFSET: f64 = -10_000.0;
const MAX_ELEMENT_OFFSET: f64 = 10_000.0;
const ALLOWED_ELEMENT_ACTIONS: &[&str] = &[
    "AXPress",
    "AXShowMenu",
    "AXIncrement",
    "AXDecrement",
    "AXConfirm",
    "AXCancel",
    "AXRaise",
    "AXShowAlternateUI",
    "AXShowDefaultUI",
];
const ALLOWED_WINDOW_ACTIONS: &[&str] = &[
    "raise",
    "minimize",
    "zoom",
    "enter_full_screen",
    "exit_full_screen",
    "toggle_full_screen",
];
const INSTRUCTION_FILE_NAMES: &[&str] = &[
    "NOLA.md",
    "NOLA.local.md",
    "AGENTS.override.md",
    "AGENTS.md",
];
const INSTRUCTION_REL_PATHS: &[&str] = &[];
const INSTRUCTION_RULE_DIRS: &[&str] = &[".nola/rules"];
const AGENT_DIRS: &[&str] = &[
    ".claude/agents",
    ".opencode/agent",
    ".opencode/agents",
    "agent",
    "agents",
];
const COMMAND_DIRS: &[&str] = &[
    ".claude/commands",
    ".opencode/command",
    ".opencode/commands",
    "command",
    "commands",
];
const AGENT_CONFIG_REL_PATHS: &[&str] = &[
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".claude/settings.jsonc",
    ".mcp.json",
    ".claude.json",
    "opencode.json",
    "opencode.jsonc",
    ".opencode.json",
    ".opencode.jsonc",
    ".opencode/opencode.json",
    ".opencode/opencode.jsonc",
    ".opencode/tui.json",
];
const AGENT_EXTENSION_DIRS: &[&str] = &[
    ".opencode/tool",
    ".opencode/tools",
    ".opencode/plugin",
    ".opencode/plugins",
    ".claude/plugins",
];
const AGENT_EXTENSION_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "jsonc", "md",
];
const CI_DIRECT_REL_PATHS: &[&str] = &[
    ".gitlab-ci.yml",
    ".gitlab-ci.yaml",
    "azure-pipelines.yml",
    "azure-pipelines.yaml",
    "bitbucket-pipelines.yml",
    "bitbucket-pipelines.yaml",
    ".circleci/config.yml",
    ".circleci/config.yaml",
    "Makefile",
    "makefile",
    "GNUmakefile",
    "justfile",
    "Justfile",
    "Taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yml",
    "taskfile.yaml",
];
const CI_WORKFLOW_DIRS: &[&str] = &[".github/workflows"];
const VERIFY_WORDS: &[&str] = &[
    "验证",
    "校验",
    "测试",
    "test",
    "verify",
    "validation",
    "build",
];
const READ_ONLY_COMMANDS: &[&str] = &[
    "cat",
    "cd",
    "cut",
    "diff",
    "du",
    "file",
    "find",
    "git",
    "grep",
    "head",
    "ls",
    "md5sum",
    "nl",
    "od",
    "pwd",
    "rg",
    "sed",
    "sha1sum",
    "sha256sum",
    "sort",
    "stat",
    "strings",
    "tail",
    "test",
    "tree",
    "uniq",
    "wc",
];
const WRITE_COMMANDS: &[&str] = &[
    "cargo", "chmod", "chown", "cp", "go", "gradle", "make", "mkdir", "mv", "npm", "pnpm",
    "python", "python3", "swift", "task", "touch", "yarn",
];
const DESTRUCTIVE_HEADS: &[&str] = &["kill", "killall", "pkill", "rm", "rmdir", "shred"];
const NETWORK_HEADS: &[&str] = &[
    "curl", "gh", "git", "npm", "pnpm", "pip", "python", "python3", "wget", "yarn",
];

#[derive(Debug, Default)]
struct PermissionCache {
    scope: Option<String>,
    persistent_grants: bool,
    config_path: Option<PathBuf>,
    allowed_dirs: Vec<PathBuf>,
    tool_rules: Vec<String>,
    bash_prefixes: Vec<String>,
    network_rules: Vec<String>,
    agent_rules: Vec<String>,
    unrestricted_resources: bool,
}

static SESSION_PERMISSION_CACHES: OnceLock<Mutex<HashMap<String, PermissionCache>>> =
    OnceLock::new();
static PERMISSION_SCOPE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostPermissionResolution {
    /// The exact canonical Run already committed its cancellation.
    StopRequested,
    TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason),
    Allow,
    AllowConversation,
    Custom(String),
    ReturnToComposer,
    Deny,
}

pub type HostPermissionResolver =
    Arc<dyn Fn(&ToolRequest, &[nola_core::agent::host::PermissionInstruction], bool) -> HostPermissionResolution + Send + Sync>;
pub type DeviceBridgeResolver =
    Arc<dyn Fn(&ToolRequest, &DeviceBridgeRuntimeTool) -> ToolOutcome + Send + Sync>;

/// Reads the only cross-runtime Artifact extension accepted by Nola Hosts.
///
/// External runtimes must return canonical HostArtifact objects under
/// `_meta["nola/artifacts"]`. Arbitrary strings, paths in prose, and fields
/// with similar names are deliberately ignored so a result cannot acquire
/// filesystem or remote-resource authority by accident.
pub(crate) fn explicit_host_artifacts_from_value(
    value: &Value,
) -> Result<Vec<nola_core::agent::host::HostArtifact>, String> {
    let Some(raw) = value.pointer("/_meta/nola~1artifacts") else {
        return Ok(Vec::new());
    };
    let artifacts = serde_json::from_value::<Vec<nola_core::agent::host::HostArtifact>>(
        raw.clone(),
    )
    .map_err(|error| format!("invalid _meta[nola/artifacts] contract: {error}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut validated = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        artifact.validate()?;
        if seen.insert(artifact.uri.clone()) {
            validated.push(artifact);
        }
    }
    Ok(validated)
}


thread_local! {
    static CURRENT_EXECUTION_PERMISSION_PROFILE: std::cell::RefCell<crate::execution_context::ScopeStack<Option<nola_domain::PermissionProfile>>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_PERMISSION_SCOPE: std::cell::RefCell<crate::execution_context::ScopeStack<String>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_FILE_READ_SCOPE: std::cell::RefCell<crate::execution_context::ScopeStack<String>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_SOP_INVOCATION: std::cell::RefCell<crate::execution_context::ScopeStack<Option<nola_core::agent::ToolInvocationContext>>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
}

/// One physical invocation's policy. This never mutates the shared session
/// cache, so a policy edit cannot change the authority of an in-flight effect.
pub(crate) struct ExecutionPermissionProfileGuard {
    binding: u64,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for ExecutionPermissionProfileGuard {
    fn drop(&mut self) {
        CURRENT_EXECUTION_PERMISSION_PROFILE.with(|scope| scope.borrow_mut().remove(self.binding));
    }
}

pub(crate) fn enter_execution_permission_profile(
    profile: Option<nola_domain::PermissionProfile>,
) -> ExecutionPermissionProfileGuard {
    let binding = CURRENT_EXECUTION_PERMISSION_PROFILE.with(|scope| scope.borrow_mut().push(profile));
    ExecutionPermissionProfileGuard { binding, thread_bound: Default::default() }
}

#[cfg(test)]
mod effect_permission_policy_tests {
    use super::*;

    #[test]
    fn effect_policy_is_nested_and_does_not_rewrite_session_defaults() {
        let scope = SessionPermissionScope::default();
        let _scope = scope.enter();
        scope.configure_profile(nola_domain::PermissionProfile::Cautious);
        assert!(!unrestricted_resource_policy());
        {
            let _effect = enter_execution_permission_profile(Some(nola_domain::PermissionProfile::AllowAll));
            assert!(unrestricted_resource_policy());
            scope.configure_profile(nola_domain::PermissionProfile::Cautious);
            assert!(unrestricted_resource_policy(),"session edits cannot downgrade an already admitted effect");
            {
                let _nested = enter_execution_permission_profile(Some(nola_domain::PermissionProfile::Cautious));
                assert!(!unrestricted_resource_policy());
            }
            assert!(unrestricted_resource_policy());
        }
        assert!(!unrestricted_resource_policy());
    }

    #[test]
    fn concurrent_effects_can_have_different_profiles_for_the_same_owner() {
        let scope = SessionPermissionScope::default();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|threads| {
            for profile in nola_domain::PermissionProfile::ALL {
                let scope = scope.clone();
                let barrier = barrier.clone();
                threads.spawn(move || {
                    let _scope = scope.enter();
                    let _effect = enter_execution_permission_profile(Some(profile));
                    barrier.wait();
                    assert_eq!(unrestricted_resource_policy(),profile == nola_domain::PermissionProfile::AllowAll);
                });
            }
        });
    }
}

pub(crate) struct SopInvocationScopeGuard {
    binding: u64,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for SopInvocationScopeGuard {
    fn drop(&mut self) {
        CURRENT_SOP_INVOCATION.with(|current| current.borrow_mut().remove(self.binding));
    }
}

pub(crate) fn enter_sop_invocation_scope(
    invocation: Option<nola_core::agent::ToolInvocationContext>,
) -> SopInvocationScopeGuard {
    let binding = CURRENT_SOP_INVOCATION.with(|current| current.borrow_mut().push(invocation));
    SopInvocationScopeGuard { binding, thread_bound: Default::default() }
}

pub(crate) fn current_sop_invocation_context() -> Option<nola_core::agent::ToolInvocationContext> {
    CURRENT_SOP_INVOCATION.with(|current| current.borrow().current().cloned().flatten())
}

struct HostPermissionSessionBinding {
    session_id: String,
    blocked: std::cell::Cell<bool>,
}

struct HostPermissionResolverBinding {
    session: Option<std::rc::Rc<HostPermissionSessionBinding>>,
    resolver: HostPermissionResolver,
}

struct DeviceBridgeResolverBinding {
    session: Option<std::rc::Rc<HostPermissionSessionBinding>>,
    identity: nola_core::tool_policy::RuntimeIdentity,
    resolver: DeviceBridgeResolver,
}

struct DeviceBridgeToolsBinding {
    session: Option<std::rc::Rc<HostPermissionSessionBinding>>,
    tools: HashMap<String, DeviceBridgeRuntimeTool>,
}

fn permission_owner_is_current(owner: &Option<std::rc::Rc<HostPermissionSessionBinding>>) -> bool {
    CURRENT_HOST_PERMISSION_SESSION.with(|current| match (owner.as_ref(), current.borrow().current()) {
        (Some(bound), Some(active)) => std::rc::Rc::ptr_eq(bound, active),
        (None, None) => true,
        _ => false,
    })
}

thread_local! {
    static CURRENT_HOST_PERMISSION_SESSION: std::cell::RefCell<crate::execution_context::ScopeStack<std::rc::Rc<HostPermissionSessionBinding>>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_DEVICE_BRIDGE_TOOLS: std::cell::RefCell<crate::execution_context::ScopeStack<DeviceBridgeToolsBinding>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_DEVICE_BRIDGE_RESOLVER: std::cell::RefCell<crate::execution_context::ScopeStack<DeviceBridgeResolverBinding>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static CURRENT_HOST_PERMISSION_RESOLVER: std::cell::RefCell<crate::execution_context::ScopeStack<HostPermissionResolverBinding>> =
        const { std::cell::RefCell::new(crate::execution_context::ScopeStack::new()) };
    static PENDING_CUSTOM_PERMISSION: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

static PENDING_NATIVE_PERMISSION_TRACES: OnceLock<Mutex<HashMap<String, PendingPermissionTraces>>> =
    OnceLock::new();

pub struct HostPermissionResolverGuard {
    binding: u64,
    // Enter/drop belong to the same thread-local scope, even though the
    // explicitly captured resolver itself may be passed to a Host worker.
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for HostPermissionResolverGuard {
    fn drop(&mut self) {
        CURRENT_HOST_PERMISSION_RESOLVER.with(|resolver| resolver.borrow_mut().remove(self.binding));
    }
}

pub fn enter_host_permission_resolver(
    resolver: HostPermissionResolver,
) -> HostPermissionResolverGuard {
    let session = CURRENT_HOST_PERMISSION_SESSION.with(|current| current.borrow().current().cloned());
    let binding = CURRENT_HOST_PERMISSION_RESOLVER.with(|current| current.borrow_mut().push(HostPermissionResolverBinding { session, resolver }));
    HostPermissionResolverGuard { binding, _thread_bound: std::marker::PhantomData }
}

pub fn current_host_permission_resolver() -> Option<HostPermissionResolver> {
    let session = CURRENT_HOST_PERMISSION_SESSION.with(|current| current.borrow().current().cloned());
    CURRENT_HOST_PERMISSION_RESOLVER.with(|current| {
        let current = current.borrow();
        let binding = current.current()?;
        let same_owner = match (&binding.session, &session) {
            (Some(bound), Some(active)) => std::rc::Rc::ptr_eq(bound, active),
            (None, None) => true,
            _ => false,
        };
        same_owner.then(|| Arc::clone(&binding.resolver))
    })
}

#[cfg(test)]
mod permission_resolver_scope_tests {
    use super::*;

    fn deny() -> HostPermissionResolver { Arc::new(|_, _, _| HostPermissionResolution::Deny) }

    #[test]
    fn resolver_and_session_never_cross_owners_or_revive_expired_binding() {
        let session_a = enter_host_permission_session("A");
        block_current_host_permission_session();
        let resolver_a = enter_host_permission_resolver(deny());
        let nested_a = enter_host_permission_session("A");
        assert!(current_host_permission_resolver().is_some());
        assert!(current_host_permission_session_is_blocked());
        drop(nested_a);
        let session_b = enter_host_permission_session("B");
        assert!(current_host_permission_resolver().is_none());
        assert!(!current_host_permission_session_is_blocked());
        let resolver_b = enter_host_permission_resolver(deny());
        drop(session_a);
        drop(resolver_a);
        assert_eq!(scoped_host_permission_session_id().as_deref(), Some("B"));
        assert!(current_host_permission_resolver().is_some());
        drop(session_b);
        assert!(current_host_permission_resolver().is_none());
        let new_b = enter_host_permission_session("B");
        assert!(current_host_permission_resolver().is_none(), "same string is not the expired owner binding");
        drop(new_b);
        drop(resolver_b);
        assert!(scoped_host_permission_session_id().is_none());
        assert!(!current_host_permission_session_is_blocked());
    }

    #[test]
    fn removing_inner_resolver_does_not_expose_another_sessions_outer_resolver() {
        let a = enter_host_permission_session("A");
        let ra = enter_host_permission_resolver(deny());
        let b = enter_host_permission_session("B");
        let rb = enter_host_permission_resolver(deny());
        drop(rb);
        assert!(current_host_permission_resolver().is_none());
        drop(b);
        assert!(current_host_permission_resolver().is_some());
        drop(a);
        assert!(current_host_permission_resolver().is_none());
        drop(ra);
    }

    #[test]
    fn permission_cache_and_sop_scopes_remove_only_their_own_binding() {
        let a = SessionPermissionScope::default();
        let b = SessionPermissionScope::default();
        let ga = a.enter();
        let gb = b.enter();
        let b_key = current_permission_scope_key();
        drop(ga);
        assert_eq!(current_permission_scope_key(), b_key);
        drop(gb);
        assert!(!permission_scope_is_active());
        let sop = enter_sop_invocation_scope(Some(nola_core::agent::ToolInvocationContext {
            sop_run_id: "A".into(), sop_step_id: "step".into(),
        }));
        let hidden = enter_sop_invocation_scope(None);
        assert!(current_sop_invocation_context().is_none());
        drop(sop);
        drop(hidden);
        assert!(current_sop_invocation_context().is_none());
    }

    #[test]
    fn device_resolver_and_identity_are_one_exact_binding() {
        let install = |session: &str| enter_device_bridge_resolver(
            nola_core::tool_policy::RuntimeIdentity { host_id: Some("host".into()), client_id: Some("client".into()), session_id: Some(session.into()) },
            Arc::new(|_, _| error("not executed".into())),
        ).unwrap();
        let a = install("A");
        let b = install("B");
        drop(a);
        assert_eq!(current_device_bridge_execution_identity().unwrap().session_id.as_deref(), Some("B"));
        assert!(device_bridge_resolver_available());
        drop(b);
        assert!(current_device_bridge_execution_identity().is_none());
        assert!(!device_bridge_resolver_available());
        let a = enter_device_bridge_tools(Vec::new()).unwrap();
        let b = enter_device_bridge_tools(Vec::new()).unwrap();
        drop(a);
        drop(b);
        assert!(CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| tools.borrow().current().is_none()));
    }

    #[test]
    fn device_callback_is_hidden_when_permission_session_owner_changes() {
        let a = enter_host_permission_session("A");
        let resolver = enter_device_bridge_resolver(
            nola_core::tool_policy::RuntimeIdentity { host_id: Some("host".into()), client_id: Some("client".into()), session_id: Some("device-session".into()) },
            Arc::new(|_, _| panic!("must not execute")),
        ).unwrap();
        assert!(device_bridge_resolver_available());
        let b = enter_host_permission_session("B");
        assert!(!device_bridge_resolver_available());
        assert!(current_device_bridge_execution_identity().is_none());
        drop(a);
        drop(b);
        assert!(!device_bridge_resolver_available());
        drop(resolver);
    }

    #[test]
    fn dropping_outer_resolver_cannot_resurrect_it_after_inner_exit() {
        assert!(current_host_permission_resolver().is_none());
        let outer_resolver = deny();
        let inner_resolver = deny();
        let outer = enter_host_permission_resolver(outer_resolver);
        let inner = enter_host_permission_resolver(Arc::clone(&inner_resolver));
        drop(outer);
        assert!(Arc::ptr_eq(&current_host_permission_resolver().unwrap(), &inner_resolver));
        drop(inner);
        assert!(current_host_permission_resolver().is_none());
    }

    #[test]
    fn error_exit_restores_only_live_resolver_and_child_thread_requires_explicit_binding() {
        let resolver = deny();
        let outer = enter_host_permission_resolver(Arc::clone(&resolver));
        let result = (|| -> Result<(), &'static str> {
            let _inner = enter_host_permission_resolver(deny());
            Err("interrupted turn")
        })();
        assert!(result.is_err());
        assert!(Arc::ptr_eq(&current_host_permission_resolver().unwrap(), &resolver));
        std::thread::spawn(|| {
            assert!(current_host_permission_resolver().is_none());
            let binding = enter_host_permission_resolver(deny());
            assert!(current_host_permission_resolver().is_some());
            drop(binding);
            assert!(current_host_permission_resolver().is_none());
        }).join().unwrap();
        drop(outer);
        assert!(current_host_permission_resolver().is_none());
    }
}

#[derive(Default)]
struct PendingPermissionTraces {
    generation: Option<(String, String, String, String)>,
    traces: HashMap<String, Value>,
}

fn permission_trace_identity(trace: &Value) -> Option<(String, String, String, String)> {
    fn required(trace: &Value, field: &str) -> Option<String> {
        trace
            .get(field)?
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
    Some((
        required(trace, "conversation_id")?,
        required(trace, "trace_id")?,
        required(trace, "run_id")?,
        required(trace, "turn_id")?,
    ))
}

pub(crate) fn set_pending_native_permission_trace(trace: Value) {
    const MAXIMUM_PENDING_CONVERSATIONS: usize = 128;
    const MAXIMUM_PENDING_TRACES: usize = 128;
    struct TraceSize(usize);
    impl std::io::Write for TraceSize {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > 32 * 1024 {
                return Err(std::io::Error::other(
                    "permission trace exceeds admission budget",
                ));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if trace.get("kind").and_then(Value::as_str) != Some("tool.permission.requested")
        || serde_json::to_writer(TraceSize(0), &trace).is_err()
    {
        return;
    }
    let Some(generation) = permission_trace_identity(&trace) else {
        return;
    };
    let Some(sequence) = trace.get("sequence").and_then(Value::as_u64) else {
        return;
    };
    let Some(payload) = trace.get("payload") else {
        return;
    };
    let Some(request_id) = payload
        .get("permission_request_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let Some(call_id) = payload
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    if payload
        .get("tool_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .is_none()
        || trace
            .get("span_id")
            .filter(|id| !id.is_null())
            .is_some_and(|id| id.as_str() != Some(call_id))
    {
        return;
    }
    let conversation_id = generation.0.clone();
    let request_id = request_id.to_string();
    let Ok(mut conversations) = PENDING_NATIVE_PERMISSION_TRACES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    else {
        return;
    };
    if !conversations.contains_key(&conversation_id)
        && conversations.len() >= MAXIMUM_PENDING_CONVERSATIONS
    {
        return;
    }
    {
        let pending = conversations.entry(conversation_id).or_default();
        if pending
            .generation
            .as_ref()
            .is_some_and(|current| current != &generation)
        {
            if pending.traces.contains_key(&request_id) {
                return;
            }
            pending.traces.clear();
        }
        pending.generation = Some(generation);
        if let Some(previous) = pending.traces.get(&request_id) {
            if previous
                .get("sequence")
                .and_then(Value::as_u64)
                .is_some_and(|old| old >= sequence)
                || previous.pointer("/payload/call_id") != trace.pointer("/payload/call_id")
                || previous.pointer("/payload/tool_name") != trace.pointer("/payload/tool_name")
            {
                return;
            }
        } else if pending.traces.len() >= MAXIMUM_PENDING_TRACES {
            return;
        }
        pending.traces.insert(request_id, trace);
    }
}

pub(crate) fn take_pending_host_permission_trace(
    expected_conversation_id: &str,
    request: &ToolRequest,
    args: &Value,
    reason: &str,
) -> Option<Value> {
    let expected_conversation_id = expected_conversation_id.trim();
    if expected_conversation_id.is_empty() {
        return None;
    }
    let mut conversations = PENDING_NATIVE_PERMISSION_TRACES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    let (mut trace, empty) = {
        let pending = conversations.get_mut(expected_conversation_id)?;
        let trace = pending.traces.get(&request.id)?;
        if trace.get("conversation_id").and_then(Value::as_str) != Some(expected_conversation_id) {
            return None;
        }
        let payload = trace.get("payload")?;
        if payload.get("permission_request_id").and_then(Value::as_str) != Some(request.id.as_str())
            || payload.get("tool_name").and_then(Value::as_str) != Some(request.name.as_str())
        {
            return None;
        }
        let call_id = payload
            .get("call_id")?
            .as_str()
            .filter(|id| !id.is_empty())?;
        if trace
            .get("span_id")
            .filter(|id| !id.is_null())
            .is_some_and(|id| id.as_str() != Some(call_id))
        {
            return None;
        }
        let request_id = request.id.clone();
        let trace = pending.traces.remove(&request_id)?;
        (trace, pending.traces.is_empty())
    };
    if empty {
        conversations.remove(expected_conversation_id);
    }
    let payload = trace.get_mut("payload")?.as_object_mut()?;
    // Supplement the canonical event with the actual immutable call input;
    // never manufacture its ask ID, call ID, tool name or event kind.
    payload.insert("arguments".to_string(), args.clone());
    payload.insert("reason".to_string(), Value::String(reason.to_string()));
    Some(trace)
}

pub struct DeviceBridgeResolverGuard {
    binding: u64,
    // Drop restores thread-local authority and must run on the owning thread.
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for DeviceBridgeResolverGuard {
    fn drop(&mut self) {
        CURRENT_DEVICE_BRIDGE_RESOLVER.with(|resolver| resolver.borrow_mut().remove(self.binding));
    }
}

pub fn enter_device_bridge_resolver(
    identity: nola_core::tool_policy::RuntimeIdentity,
    resolver: DeviceBridgeResolver,
) -> Result<DeviceBridgeResolverGuard, String> {
    if [&identity.host_id, &identity.client_id, &identity.session_id].iter()
        .any(|value| value.as_deref().is_none_or(|value| value.trim().is_empty())) {
        return Err("Device resolver requires an authenticated execution identity".into());
    }
    let session = CURRENT_HOST_PERMISSION_SESSION.with(|current| current.borrow().current().cloned());
    let binding = CURRENT_DEVICE_BRIDGE_RESOLVER.with(|current| current.borrow_mut().push(DeviceBridgeResolverBinding { session, identity, resolver }));
    Ok(DeviceBridgeResolverGuard { binding, thread_bound: Default::default() })
}

pub(crate) fn current_device_bridge_execution_identity() -> Option<nola_core::tool_policy::RuntimeIdentity> {
    CURRENT_DEVICE_BRIDGE_RESOLVER.with(|current| current.borrow().current().filter(|binding| permission_owner_is_current(&binding.session)).map(|binding| binding.identity.clone()))
}

pub(crate) fn device_bridge_resolver_available() -> bool {
    current_device_bridge_resolver().is_some()
}

fn current_device_bridge_resolver() -> Option<DeviceBridgeResolver> {
    CURRENT_DEVICE_BRIDGE_RESOLVER.with(|current| current.borrow().current().filter(|binding| permission_owner_is_current(&binding.session)).map(|binding| Arc::clone(&binding.resolver)))
}

pub struct DeviceBridgeToolsGuard {
    binding: u64,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[derive(Clone, Debug)]
pub struct DeviceBridgeRuntimeTool {
    pub schema: Value,
    pub capability: String,
    pub result_egress_receipt: nola_core::tool_policy::RuntimeResultEgressReceipt,
    pub result_egress_grant: nola_core::tool_policy::ResultEgressGrant,
}

impl Drop for DeviceBridgeToolsGuard {
    fn drop(&mut self) {
        CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| tools.borrow_mut().remove(self.binding));
    }
}

pub fn enter_device_bridge_tools(
    runtime_tools: Vec<DeviceBridgeRuntimeTool>,
) -> Result<DeviceBridgeToolsGuard, String> {
    let mut mapped = HashMap::new();
    for runtime_tool in runtime_tools {
        let name = nola_core::device_bridge::device_tool_name(&runtime_tool.schema)?.to_string();
        if mapped.insert(name, runtime_tool).is_some() {
            return Err("duplicate device bridge tool".to_string());
        }
    }
    let session = CURRENT_HOST_PERMISSION_SESSION.with(|current| current.borrow().current().cloned());
    let binding = CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| tools.borrow_mut().push(DeviceBridgeToolsBinding { session, tools: mapped }));
    Ok(DeviceBridgeToolsGuard { binding, thread_bound: Default::default() })
}

pub fn current_device_bridge_runtime_tools() -> Vec<DeviceBridgeRuntimeTool> {
    CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| {
        let tools = tools.borrow();
        let Some(tools) = tools.current().filter(|binding| permission_owner_is_current(&binding.session)) else { return Vec::new(); };
        let mut entries = tools.tools.iter()
            .map(|(name, runtime_tool)| (name.clone(), runtime_tool.clone()))
            .collect::<Vec<_>>();
        entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        entries
            .into_iter()
            .map(|(_, runtime_tool)| runtime_tool)
            .collect()
    })
}

pub fn current_device_bridge_tool_schema(name: &str) -> Option<Value> {
    CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| {
        tools
            .borrow()
            .current().filter(|binding| permission_owner_is_current(&binding.session)).and_then(|binding| binding.tools.get(name))
            .map(|runtime_tool| runtime_tool.schema.clone())
    })
}

fn current_device_bridge_runtime_tool(name: &str) -> Option<DeviceBridgeRuntimeTool> {
    CURRENT_DEVICE_BRIDGE_TOOLS.with(|tools| tools.borrow().current().filter(|binding| permission_owner_is_current(&binding.session)).and_then(|binding| binding.tools.get(name)).cloned())
}

pub struct HostPermissionSessionGuard {
    binding: u64,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for HostPermissionSessionGuard {
    fn drop(&mut self) {
        CURRENT_HOST_PERMISSION_SESSION.with(|session| session.borrow_mut().remove(self.binding));
    }
}

pub fn enter_host_permission_session(session_id: &str) -> HostPermissionSessionGuard {
    let binding = CURRENT_HOST_PERMISSION_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        // Core enters the already-bound canonical owner inside the surface
        // scope. Preserve that live owner identity (and its blocked state),
        // but never search older frames by matching a session string.
        let owner = session.current().filter(|owner| owner.session_id == session_id.trim()).cloned()
            .unwrap_or_else(|| std::rc::Rc::new(HostPermissionSessionBinding {
                session_id: session_id.trim().to_string(), blocked: std::cell::Cell::new(false),
            }));
        session.push(owner)
    });
    HostPermissionSessionGuard {
        binding, thread_bound: Default::default(),
    }
}

pub(crate) fn current_host_session_id() -> Option<String> {
    scoped_host_permission_session_id()
        .or_else(crate::desktop_host_control::active_session_id)
}

fn scoped_host_permission_session_id() -> Option<String> {
    CURRENT_HOST_PERMISSION_SESSION.with(|session| session.borrow().current().map(|session| session.session_id.clone()))
}

pub fn block_current_host_permission_session() {
    CURRENT_HOST_PERMISSION_SESSION.with(|session| { if let Some(session) = session.borrow().current() { session.blocked.set(true); } });
}

pub fn reset_current_host_permission_session_block() {
    CURRENT_HOST_PERMISSION_SESSION.with(|session| { if let Some(session) = session.borrow().current() { session.blocked.set(false); } });
}

fn current_host_permission_session_is_blocked() -> bool {
    CURRENT_HOST_PERMISSION_SESSION.with(|session| session.borrow().current().is_some_and(|session| session.blocked.get()))
}

/// Routes macOS computer-use operations back to the signed Nola app process.
/// Accessibility and Screen Recording permissions are process-identity bound;
/// running these operations from a temporary Python helper makes the permission
/// status observed by the app differ from the executor. The line protocol keeps
/// the Agent Host suspended until the app returns the result for this call.
pub fn execute_mac_native_computer_bridge(
    session_id: &str,
    tool_name: &str,
    call_id: &str,
    args: &Map<String, Value>,
) -> Option<ToolOutcome> {
    if !mac_native_computer_bridge_is_available()
        || !mac_native_computer_bridge_supports(tool_name)
    {
        return None;
    }
    let event = json!({
        "type": "device_tool_request",
        "request_id": call_id,
        "session_id": session_id,
        "tool_name": tool_name,
        "arguments": args,
    });
    if crate::native_host_output::emit(event).is_err() {
        return Some(error(
            "failed to publish the Mac computer-use request".to_string(),
        ));
    }
    let response = if crate::desktop_host_control::is_active() {
        let timeout_seconds = std::env::var("NOLA_DEVICE_TOOL_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(300);
        let Some(response) = crate::desktop_host_control::wait_for_device_tool_result(
            call_id,
            Duration::from_secs(timeout_seconds),
        ) else {
            return Some(error(
                "the Mac computer-use bridge disconnected".to_string(),
            ));
        };
        response
    } else {
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).is_err() || line.trim().is_empty() {
            return Some(error(
                "the Mac computer-use bridge disconnected".to_string(),
            ));
        }
        let Ok(response) = serde_json::from_str::<Value>(&line) else {
            return Some(error(
                "the Mac computer-use bridge returned invalid JSON".to_string(),
            ));
        };
        response
    };
    decode_mac_native_bridge_response(&response, call_id)
}

fn decode_mac_native_bridge_response(response: &Value, call_id: &str) -> Option<ToolOutcome> {
    if response.get("type").and_then(Value::as_str) != Some("device_tool_result")
        || response.get("request_id").and_then(Value::as_str) != Some(call_id)
    {
        return Some(error(
            "the Mac computer-use bridge returned a mismatched result".to_string(),
        ));
    }
    let is_error = response
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let output = response.get("output").cloned().unwrap_or(Value::Null);
    let text = output
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| serde_json::to_string_pretty(&output).unwrap_or_default());
    let artifacts = match response.get("artifacts") {
        Some(Value::Array(values)) => {
            let mut artifacts = Vec::with_capacity(values.len());
            for value in values {
                let artifact = match serde_json::from_value::<
                    nola_core::agent::host::HostArtifact,
                >(value.clone()) {
                    Ok(artifact) => artifact,
                    Err(error) => {
                        return Some(crate::tools::error(format!(
                            "the Mac device bridge returned an invalid artifact: {error}"
                        )))
                    }
                };
                if let Err(error) = artifact.validate() {
                    return Some(crate::tools::error(format!(
                        "the Mac device bridge returned an invalid artifact: {error}"
                    )));
                }
                artifacts.push(artifact);
            }
            artifacts
        }
        Some(_) => {
            return Some(crate::tools::error(
                "the Mac device bridge returned a non-array artifacts field".to_string(),
            ))
        }
        None => Vec::new(),
    };
    // Evidence is a Host-owned envelope field, not text or an assertion in
    // business output. Preserve it through the native bridge into Core.
    let execution = match response.get("execution") {
        None => None,
        Some(value) => {
            let receipt = match serde_json::from_value::<nola_host_protocol::tool_result::ExecutionReceipt>(value.clone()) {
                Ok(receipt) => receipt,
                Err(error) => return Some(crate::tools::error(format!(
                    "the Mac device bridge returned an invalid execution receipt: {error}"
                ))),
            };
            if let Err(error) = receipt.validate() {
                return Some(crate::tools::error(format!(
                    "the Mac device bridge returned an invalid execution receipt: {error}"
                )));
            }
            Some(receipt)
        }
    };
    Some(ToolOutcome {
        full_output_ref: None,
        data: Some(output),
        execution,
        text,
        display: response
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or(if is_error {
                "Mac computer action failed"
            } else {
                "Mac computer action completed"
            })
            .to_string(),
        error: is_error,
        artifacts,
        activation: None,
    })
}

pub fn mac_native_computer_bridge_is_available() -> bool {
    let explicitly_enabled = std::env::var("NOLA_MAC_NATIVE_COMPUTER_BRIDGE")
        .ok()
        .as_deref()
        == Some("1");
    let authenticated_mac_host = cfg!(target_os = "macos")
        && std::env::var("NOLA_CLIENT_SURFACE").ok().as_deref() == Some("mac")
        && crate::desktop_host_control::is_active();
    explicitly_enabled || authenticated_mac_host
}

pub fn mac_native_computer_bridge_tool_ids() -> &'static [&'static str] {
    &[
        "conversation_recall",
        "nola_orchestrate",
        "search_calendar_events",
        "create_calendar_event",
        "search_reminders",
        "create_reminder",
        "update_reminder",
        "computer_status",
        "computer_observe",
        "computer_list_displays",
        "computer_screenshot",
        "computer_get_clipboard",
        "computer_set_clipboard",
        "computer_type_text",
        "computer_press_key",
        "computer_hotkey",
        "computer_key_down",
        "computer_key_up",
        "computer_copy_selection",
        "computer_accessibility_snapshot",
        "computer_read_text",
        "computer_find_text",
        "computer_click_text",
        "computer_select_text",
        "computer_element_at",
        "computer_click_element",
        "computer_press_element",
        "computer_focus_element",
        "computer_perform_element_action",
        "computer_type_element",
        "computer_set_element_value",
        "computer_assert_element",
        "computer_move_to_element",
        "computer_get_position",
        "computer_move_mouse",
        "computer_mouse_down",
        "computer_mouse_up",
        "computer_click",
        "computer_drag",
        "computer_scroll",
        "computer_press_at",
    ]
}

fn mac_native_computer_bridge_supports(name: &str) -> bool {
    mac_native_computer_bridge_tool_ids().contains(&name)
}


#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentTodo {
    id: String,
    content: String,
    active_form: String,
    status: String,
    priority: String,
    dependency_ids: Vec<String>,
    approval_status: String,
    capability_refs: Vec<String>,
    expected_artifact_refs: Vec<String>,
    artifact_refs: Vec<String>,
    work_step_transaction_refs: Vec<String>,
    attempt: u32,
    executor_ref: Option<String>,
    resume_key: Option<String>,
    success_criteria: Vec<String>,
    evidence_refs: Vec<String>,
    verification_status: String,
    termination_reason: Option<String>,
}

#[derive(Debug)]
struct BackgroundBashTask {
    #[allow(dead_code)]
    id: String,
    project_root: PathBuf,
    command: String,
    cwd: PathBuf,
    child: Option<Child>,
    pid: u32,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    status_path: PathBuf,
    started_at_ms: i64,
    owner_session_id: Option<String>,
    notify_on_completion: bool,
    output_contract: Option<BackgroundOutputContract>,
}

#[derive(Debug, Clone)]
pub struct BackgroundTaskSummary {
    pub id: String,
    /// Canonical RuntimeTask state; status below is presentation only.
    pub canonical_state: String,
    pub status: String,
    pub command: String,
    pub pid: u32,
    pub elapsed_secs: f64,
    pub owner_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BackgroundTaskNotification {
    pub summary: BackgroundTaskSummary,
    pub delivery: nola_application::RuntimeTaskNotificationDelivery,
}

impl std::ops::Deref for BackgroundTaskNotification {
    type Target = BackgroundTaskSummary;
    fn deref(&self) -> &Self::Target { &self.summary }
}

#[derive(Clone, Debug)]
struct WorkspaceCheckpoint {
    id: String,
    label: String,
    root: PathBuf,
    git_root: String,
    revision: String,
    source: String,
    created_at: u64,
    status: String,
    untracked: Vec<String>,
}

#[derive(Clone, Debug)]
struct MoveImportUpdate {
    path: PathBuf,
    original: String,
    version: FileMutationVersion,
    updated: String,
    lines: Vec<String>,
}

#[derive(Clone, Debug)]
struct FileAccessState {
    version: FileReadVersion,
    full_read: bool,
    ranges: Vec<(usize, usize)>,
}

static BACKGROUND_BASH_TASKS: OnceLock<Mutex<HashMap<String, BackgroundBashTask>>> =
    OnceLock::new();
static WORKSPACE_CHECKPOINTS: OnceLock<Mutex<HashMap<String, WorkspaceCheckpoint>>> =
    OnceLock::new();
static FILE_ACCESS_STATE: OnceLock<Mutex<HashMap<(String, PathBuf), FileAccessState>>> =
    OnceLock::new();
static FILE_WRITE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

macro_rules! tool_try {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(outcome) => return outcome,
        }
    };
}

#[derive(Clone, Default)]
pub struct AgentPlanState {
    todos: Arc<Mutex<Vec<AgentTodo>>>,
    session_id: Arc<Mutex<Option<String>>>,
    sop_identity: Arc<Mutex<Option<nola_application::ConversationPlanSopIdentity>>>,
    plan_revision: Arc<Mutex<Option<i64>>>,
    validation_evidence: Arc<Mutex<Vec<String>>>,
    mutation_pending_validation: Arc<Mutex<bool>>,
}

#[derive(Debug, Clone)]
pub struct AgentPlanSnapshot {
    pub completed: usize,
    pub total: usize,
    pub current: Option<String>,
    pub steps: Vec<AgentPlanStepSnapshot>,
}

#[cfg(test)]
mod plan_owner_binding_tests {
    use super::*;

    #[test]
    fn refresh_and_replan_wait_for_the_shared_revision_boundary() {
        for replan in [false, true] {
            let state = AgentPlanState::default();
            let guard = state.plan_revision.lock().unwrap();
            let clone = state.clone();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (finished_tx, finished_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let result = if replan { clone.replan_todos(Vec::new(), "test").map(|_| ()) }
                    else { clone.refresh_persisted_plan() };
                finished_tx.send(result).unwrap();
            });
            started_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
            assert!(matches!(finished_rx.recv_timeout(std::time::Duration::from_millis(20)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
            drop(guard);
            let result = finished_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
            assert_eq!(result.is_err(), replan); // Unbound replan is rejected without touching Store.
            worker.join().unwrap();
        }
    }

    #[test]
    fn clones_cannot_rebind_a_plan_to_another_conversation() {
        let state = AgentPlanState::default();
        *state.session_id.lock().unwrap() = Some("owner".into());
        *state.plan_revision.lock().unwrap() = Some(7);
        let clone = state.clone();
        assert!(clone.bind_session("other").unwrap_err().contains("another conversation"));
        assert_eq!(clone.bind_session("owner").unwrap(), true);
        assert_eq!(state.session_id.lock().unwrap().as_deref(), Some("owner"));
        assert_eq!(*state.plan_revision.lock().unwrap(), Some(7));
        assert!(clone.bind_session(" owner ").is_err());
        assert!(AgentPlanState::default().bind_session("").is_err());
    }
}

#[derive(Debug, Clone)]
pub struct AgentPlanStepSnapshot {
    pub id: String,
    pub content: String,
    pub active_form: String,
    pub status: String,
    pub priority: String,
    pub dependency_ids: Vec<String>,
    pub approval_status: String,
}

impl AgentPlanState {
    fn clear_validation_evidence(&self) {
        if let Ok(mut evidence) = self.validation_evidence.lock() {
            evidence.clear();
        }
    }

    fn record_mutation(&self) {
        self.clear_validation_evidence();
        if let Ok(mut pending) = self.mutation_pending_validation.lock() {
            *pending = true;
        }
    }

    fn record_validation_evidence(&self, evidence: String) {
        if let Ok(mut items) = self.validation_evidence.lock() {
            items.clear();
            items.push(evidence);
        }
        if let Ok(mut pending) = self.mutation_pending_validation.lock() {
            *pending = false;
        }
    }

    fn validation_evidence(&self) -> Vec<String> {
        self.validation_evidence
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    fn mutation_pending_validation(&self) -> bool {
        self.mutation_pending_validation
            .lock()
            .map(|pending| *pending)
            .unwrap_or(true)
    }

    pub fn bind_session(&self, session_id: &str) -> Result<bool, String> {
        if session_id.trim().is_empty() || session_id != session_id.trim() {
            return Err("plan conversation identity must be nonempty and canonical".into());
        }
        // Same lock order as replace_todos. Hold the revision lock across
        // binding so clones cannot adopt different owners concurrently.
        let mut revision = self.plan_revision.lock().map_err(|_| "plan revision lock poisoned")?;
        {
            let bound = self.session_id.lock().map_err(|_| "todo session binding lock poisoned")?;
            if let Some(owner) = bound.as_deref() {
                if owner != session_id { return Err("plan state is already bound to another conversation".into()); }
                return Ok(revision.is_some());
            }
        }
        let mut application = CliApplicationPort::persistent_default()?;
        let persisted: Option<nola_application::ConversationPlan> =
            application.execute(nola_application::ApplicationCommand::ConversationPlan {
                command: nola_application::ConversationPlanCommand::Get {
                    conversation_id: session_id.to_string(),
                },
            })?;
        let restored = persisted.is_some();
        let next_revision = persisted.as_ref().map(|plan|plan.revision);
        let (identity, todos) = if let Some(plan) = persisted {
            let mut args = Map::new();
            args.insert("todos".into(), serde_json::to_value(plan.todos).map_err(|error|error.to_string())?);
            let todos = normalize_todos(&args).map_err(|outcome|outcome.text)?;
            (plan.sop, todos)
        } else {
            let definition = crate::host_prompts::current_host_sops().into_iter().next();
            let identity = nola_application::ConversationPlanSopIdentity {
                run_id: uuid::Uuid::new_v4().to_string(),
                sop_id: definition
                    .as_ref()
                    .map(|definition| definition.id.clone())
                    .unwrap_or_else(|| "nola.agent.plan".to_string()),
                sop_version: definition.as_ref().map_or(1, |definition| definition.version),
            };
            (identity, Vec::new())
        };
        // Validate everything before publication. Readers use todos -> SOP.
        let mut state = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?;
        let mut sop = self.sop_identity.lock().map_err(|_| "plan SOP identity lock poisoned")?;
        let mut bound = self.session_id.lock().map_err(|_| "todo session binding lock poisoned")?;
        *state = todos;
        *sop = Some(identity);
        *revision = next_revision;
        *bound = Some(session_id.to_string());
        self.clear_validation_evidence();
        if let Ok(mut pending) = self.mutation_pending_validation.lock() { *pending = false; }
        Ok(restored)
    }

    pub fn restore_from_args(&self, args: &Map<String, Value>) -> Result<String, String> {
        let todos = normalize_todos(args).map_err(|outcome| outcome.text)?;
        let rendered = format_todos(&todos);
        self.replace_todos(todos)?;
        Ok(rendered)
    }

    pub fn restored_context(&self) -> Result<Option<String>, String> {
        self.refresh_persisted_plan()?;
        let todos = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?;
        if todos.is_empty() {
            return Ok(None);
        }
        let Some(sop) = self
            .sop_identity
            .lock()
            .map_err(|_| "plan SOP identity lock poisoned".to_string())?
            .clone()
        else {
            return Ok(None);
        };
        Ok(Some(format!(
            "sop_run:\n  run_id: {}\n  sop_id: {}\n  sop_version: {}\n{}",
            sop.run_id,
            sop.sop_id,
            sop.sop_version,
            format_todos(&todos)
        )))
    }

    fn refresh_persisted_plan(&self) -> Result<(), String> {
        // Refresh, save, replan and first binding share one serialization
        // boundary. Do not publish a read started before a local commit.
        let mut revision = self.plan_revision.lock().map_err(|_| "plan revision lock poisoned")?;
        let Some(session_id) = self
            .session_id
            .lock()
            .map_err(|_| "todo session binding lock poisoned".to_string())?
            .clone()
        else {
            return Ok(());
        };
        let mut application = CliApplicationPort::persistent_default()?;
        let Some(plan): Option<nola_application::ConversationPlan> =
            application.execute(nola_application::ApplicationCommand::ConversationPlan {
                command: nola_application::ConversationPlanCommand::Get {
                    conversation_id: session_id.clone(),
                },
            })?
        else {
            return if revision.is_some() {
                Err("persisted conversation plan disappeared; refusing stale local state".into())
            } else { Ok(()) };
        };
        let mut args = Map::new();
        args.insert(
            "todos".to_string(),
            serde_json::to_value(&plan.todos).map_err(|error| error.to_string())?,
        );
        let todos = normalize_todos(&args).map_err(|outcome| outcome.text)?;
        if revision.is_some_and(|observed| plan.revision < observed) {
            return Err("persisted conversation plan revision moved backwards".into());
        }
        let mut state = self.todos.lock().map_err(|_| "todo state lock poisoned")?;
        let mut sop = self.sop_identity.lock().map_err(|_| "plan SOP identity lock poisoned")?;
        *state = todos;
        *sop = Some(plan.sop);
        *revision = Some(plan.revision);
        Ok(())
    }

    fn replace_todos(&self, todos: Vec<AgentTodo>) -> Result<Vec<AgentTodo>, String> {
        // Serialize local writers together with their observed version. A failed
        // commit leaves both the editor contents and read version intact.
        let mut revision = self.plan_revision.lock().map_err(|_| "plan revision lock poisoned")?;
        let session_id = self
            .session_id
            .lock()
            .map_err(|_| "todo session binding lock poisoned".to_string())?
            .clone();
        if let Some(session_id) = session_id {
            let sop = self
                .sop_identity
                .lock()
                .map_err(|_| "plan SOP identity lock poisoned".to_string())?
                .clone()
                .ok_or_else(|| "an active SOP identity is required to persist a plan".to_string())?;
            let mut application = CliApplicationPort::persistent_default()?;
            let committed: nola_application::ConversationPlan = application.execute(
                nola_application::ApplicationCommand::ConversationPlan {
                    command: nola_application::ConversationPlanCommand::Save {
                        conversation_id: session_id,
                        expected_revision: *revision,
                        sop,
                        todos: agent_todos_json(&todos),
                        updated_at_ms: chrono::Utc::now().timestamp_millis(),
                    },
                },
            )?;
            *revision = Some(committed.revision);
        }
        let mut state = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?;
        Ok(std::mem::replace(&mut *state, todos))
    }

    fn replan_todos(&self, todos: Vec<AgentTodo>, reason: &str) -> Result<Vec<AgentTodo>, String> {
        let mut revision = self.plan_revision.lock().map_err(|_| "plan revision lock poisoned")?;
        let session_id = self
            .session_id
            .lock()
            .map_err(|_| "todo session binding lock poisoned".to_string())?
            .clone()
            .ok_or_else(|| "an active session is required for explicit replanning".to_string())?;
        let mut application = CliApplicationPort::persistent_default()?;
        let current: nola_application::ConversationPlan = application
            .execute::<Option<nola_application::ConversationPlan>>(
                nola_application::ApplicationCommand::ConversationPlan {
                    command: nola_application::ConversationPlanCommand::Get {
                        conversation_id: session_id.clone(),
                    },
                },
            )?
            .ok_or_else(|| {
                "the current session does not have a persisted execution plan".to_string()
            })?;
        let next_sop = nola_application::ConversationPlanSopIdentity {
            run_id: uuid::Uuid::new_v4().to_string(),
            sop_id: current.sop.sop_id.clone(),
            sop_version: current.sop.sop_version,
        };
        let replanned: nola_application::ConversationPlan =
            application.execute(nola_application::ApplicationCommand::ConversationPlan {
                command: nola_application::ConversationPlanCommand::Replan {
                    conversation_id: session_id,
                    expected_revision: current.revision,
                    sop: next_sop,
                    todos: agent_todos_json(&todos),
                    reason: reason.to_string(),
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            })?;
        let mut state = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?;
        let mut sop = self.sop_identity.lock().map_err(|_| "plan SOP identity lock poisoned")?;
        *sop = Some(replanned.sop);
        *revision = Some(replanned.revision);
        Ok(std::mem::replace(&mut *state, todos))
    }

    fn current_todos(&self) -> Result<Vec<AgentTodo>, String> {
        self.todos
            .lock()
            .map(|todos| todos.clone())
            .map_err(|_| "todo state lock poisoned".to_string())
    }

    pub fn snapshot(&self) -> Option<AgentPlanSnapshot> {
        let todos = self.todos.lock().ok()?;
        if todos.is_empty() {
            return None;
        }
        let completed = todos
            .iter()
            .filter(|todo| todo.status == "completed")
            .count();
        let current = todos
            .iter()
            .find(|todo| todo.status == "in_progress")
            .map(|todo| {
                if todo.active_form.trim().is_empty() {
                    todo.content.clone()
                } else {
                    todo.active_form.clone()
                }
            });
        Some(AgentPlanSnapshot {
            completed,
            total: todos.len(),
            current,
            steps: todos
                .iter()
                .map(|todo| AgentPlanStepSnapshot {
                    id: todo.id.clone(),
                    content: todo.content.clone(),
                    active_form: todo.active_form.clone(),
                    status: todo.status.clone(),
                    priority: todo.priority.clone(),
                    dependency_ids: todo.dependency_ids.clone(),
                    approval_status: todo.approval_status.clone(),
                })
                .collect(),
        })
    }

    pub fn ensure_validation_pending(&self) -> Result<bool, String> {
        let mut todos = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?
            .clone();
        if todos.is_empty()
            || todos
                .iter()
                .any(|todo| matches!(todo.status.as_str(), "pending" | "in_progress"))
            || !todos.iter().any(|todo| todo.status == "completed")
        {
            return Ok(false);
        }
        todos.push(AgentTodo {
            id: "verify-result".to_string(),
            content: "Verify the result before completion".to_string(),
            active_form: "Verifying the result".to_string(),
            status: "in_progress".to_string(),
            priority: "high".to_string(),
            dependency_ids: todos
                .last()
                .map(|todo| vec![todo.id.clone()])
                .unwrap_or_default(),
            approval_status: "not_required".to_string(),
            capability_refs: Vec::new(),
            expected_artifact_refs: Vec::new(),
            artifact_refs: Vec::new(),
            work_step_transaction_refs: Vec::new(),
            attempt: 1,
            executor_ref: None,
            resume_key: None,
            success_criteria: vec!["A real validation check records its result".to_string()],
            evidence_refs: Vec::new(),
            verification_status: "pending".to_string(),
            termination_reason: None,
        });
        self.replace_todos(todos)?;
        Ok(true)
    }

    pub fn set_plan_approval(&self, step_ids: &[String], approved: bool) -> Result<usize, String> {
        let session_id = self
            .session_id
            .lock()
            .map_err(|_| "todo session binding lock poisoned".to_string())?
            .clone();
        if let Some(session_id) = session_id {
            let mut application = CliApplicationPort::persistent_default()?;
            let current = application
                .execute::<Option<nola_application::ConversationPlan>>(
                    nola_application::ApplicationCommand::ConversationPlan {
                        command: nola_application::ConversationPlanCommand::Get {
                            conversation_id: session_id.clone(),
                        },
                    },
                )?
                .ok_or_else(|| "conversation plan not found".to_string())?;
            let changed = if step_ids.is_empty() {
                current
                    .todos
                    .iter()
                    .filter(|step| {
                        step.status == nola_application::ConversationPlanStepStatus::Pending
                            && step.approval_status
                                == nola_application::ConversationPlanApprovalStatus::Pending
                    })
                    .count()
            } else {
                step_ids.len()
            };
            let saved: nola_application::ConversationPlan =
                application.execute(nola_application::ApplicationCommand::ConversationPlan {
                    command: nola_application::ConversationPlanCommand::SetApproval {
                        conversation_id: session_id,
                        expected_revision: current.revision,
                        step_ids: step_ids.to_vec(),
                        decision: if approved {
                            nola_application::ConversationPlanApprovalStatus::Approved
                        } else {
                            nola_application::ConversationPlanApprovalStatus::Rejected
                        },
                        updated_at_ms: chrono::Utc::now().timestamp_millis(),
                    },
                })?;
            let value = serde_json::to_value(saved.todos).map_err(|error| error.to_string())?;
            let mut args = Map::new();
            args.insert("todos".to_string(), value);
            let todos = normalize_todos(&args).map_err(|outcome| outcome.text)?;
            *self
                .todos
                .lock()
                .map_err(|_| "todo state lock poisoned".to_string())? = todos;
            return Ok(changed);
        }
        let mut todos = self
            .todos
            .lock()
            .map_err(|_| "todo state lock poisoned".to_string())?
            .clone();
        let selected = if step_ids.is_empty() {
            todos
                .iter()
                .filter(|todo| todo.status == "pending" && todo.approval_status == "pending")
                .map(|todo| todo.id.clone())
                .collect::<std::collections::BTreeSet<_>>()
        } else {
            step_ids.iter().cloned().collect()
        };
        if selected.is_empty() {
            return Err("no pending plan steps selected".to_string());
        }
        let known = todos
            .iter()
            .map(|todo| todo.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(unknown) = selected.iter().find(|id| !known.contains(id.as_str())) {
            return Err(format!("unknown plan step: {unknown}"));
        }
        let mut changed = 0usize;
        for todo in &mut todos {
            if !selected.contains(&todo.id) {
                continue;
            }
            if todo.status != "pending" || todo.approval_status != "pending" {
                return Err(format!("plan step {} is not awaiting approval", todo.id));
            }
            if approved {
                todo.approval_status = "approved".to_string();
            } else {
                todo.approval_status = "rejected".to_string();
                todo.status = "cancelled".to_string();
                todo.termination_reason = Some("cancelled".to_string());
            }
            changed += 1;
        }
        self.replace_todos(todos)?;
        Ok(changed)
    }
}

fn agent_todos_json(todos: &[AgentTodo]) -> Value {
    Value::Array(
        todos
            .iter()
            .map(|todo| {
                json!({
                    "id": todo.id,
                    "content": todo.content,
                    "activeForm": todo.active_form,
                    "status": todo.status,
                    "priority": todo.priority,
                    "dependencyIds": todo.dependency_ids,
                    "approvalStatus": todo.approval_status,
                    "capabilityRefs": todo.capability_refs,
                    "expectedArtifactRefs": todo.expected_artifact_refs,
                    "artifactRefs": todo.artifact_refs,
                    "workStepTransactionRefs": todo.work_step_transaction_refs,
                    "attempt": todo.attempt,
                    "executorRef": todo.executor_ref,
                    "resumeKey": todo.resume_key,
                    "successCriteria": todo.success_criteria,
                    "evidenceRefs": todo.evidence_refs,
                    "verificationStatus": todo.verification_status,
                    "terminationReason": todo.termination_reason,
                })
            })
            .collect(),
    )
}

#[derive(Clone)]
pub struct SessionPermissionScope {
    inner: Arc<SessionPermissionScopeInner>,
    // Cloned jobs retain their admitted read generation, independently of the
    // warm Host's reusable session approval cache.
    file_read_scope: Arc<FileReadEvidenceScope>,
}

struct SessionPermissionScopeInner {
    key: String,
    owner_session_id: Mutex<Option<String>>,
    resolved_resource_grants: Mutex<std::collections::BTreeSet<String>>,
}

impl Default for SessionPermissionScope {
    fn default() -> Self {
        let sequence = PERMISSION_SCOPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            file_read_scope: Arc::new(FileReadEvidenceScope::new(None)),
            inner: Arc::new(SessionPermissionScopeInner {
                key: format!("run-{}-{sequence:x}", std::process::id()),
                owner_session_id: Mutex::new(None),
                resolved_resource_grants: Mutex::new(Default::default()),
            }),
        }
    }
}

impl fmt::Debug for SessionPermissionScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionPermissionScope")
            .field("key", &self.inner.key)
            .finish()
    }
}

impl Drop for SessionPermissionScopeInner {
    fn drop(&mut self) {
        if let Some(caches) = SESSION_PERMISSION_CACHES.get() {
            if let Ok(mut caches) = caches.lock() {
                caches.remove(&self.key);
            }
        }
    }
}

pub struct SessionPermissionScopeGuard {
    binding: u64,
    file_read_binding: u64,
    _file_read_scope: Arc<FileReadEvidenceScope>,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for SessionPermissionScopeGuard {
    fn drop(&mut self) {
        CURRENT_PERMISSION_SCOPE.with(|scope| scope.borrow_mut().remove(self.binding));
        CURRENT_FILE_READ_SCOPE.with(|scope| scope.borrow_mut().remove(self.file_read_binding));
    }
}

impl SessionPermissionScope {
    pub(crate) fn bind_file_read_run(&mut self, run_id: &str) {
        if self.file_read_scope.run_id.as_deref() != Some(run_id) {
            self.file_read_scope = Arc::new(FileReadEvidenceScope::new(Some(run_id.to_owned())));
        }
    }

    pub(crate) fn configure_profile(&self, profile: crate::permissions::PermissionProfile) {
        let _scope = self.enter();
        with_permission_cache_mut(|cache| {
            cache.unrestricted_resources = profile == crate::permissions::PermissionProfile::AllowAll;
        });
    }

    pub fn bind_owner_session(&self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if let Ok(mut owner) = self.inner.owner_session_id.lock() {
            if owner.as_ref() != Some(&session_id) {
                if let Ok(mut grants) = self.inner.resolved_resource_grants.lock() { grants.clear(); }
            }
            *owner = Some(session_id);
        }
    }

    pub fn owner_session_id(&self) -> Option<String> {
        self.inner
            .owner_session_id
            .lock()
            .ok()
            .and_then(|owner| owner.clone())
    }

    pub fn enter(&self) -> SessionPermissionScopeGuard {
        let key = self.inner.key.clone();
        let binding = CURRENT_PERMISSION_SCOPE.with(|scope| scope.borrow_mut().push(key));
        let file_read_binding = CURRENT_FILE_READ_SCOPE.with(|scope|
            scope.borrow_mut().push(self.file_read_scope.key.clone()));
        SessionPermissionScopeGuard { binding, file_read_binding,
            _file_read_scope: Arc::clone(&self.file_read_scope), thread_bound: Default::default() }
    }
}

#[derive(Clone)]
pub struct ToolContext {
    pub memory_binding: nola_domain::memory_owner::MemoryExecutionBinding,
    pub language_services: Vec<nola_domain::language_service::LanguageServiceConfiguration>,
    pub output_store: ToolOutputStore,
    pub skills: SkillRuntimeContext,
    pub allowed_dirs: Vec<PathBuf>,
    pub policy: ToolExecutionPolicy,
    pub permission_scope: SessionPermissionScope,
    pub cancellation: RuntimeExecutionControl,
    pub plan_state: AgentPlanState,
    pub browser: BrowserRuntime,
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolContext")
            .field("skills", &self.skills)
            .field("allowed_dirs", &self.allowed_dirs)
            .field("policy", &self.policy)
            .field("permission_scope", &self.permission_scope)
            .field("cancellation", &self.cancellation)
            .field("browser", &self.browser)
            .finish()
    }
}

#[derive(Clone)]
pub struct BrowserRuntime {
    state_dir: PathBuf,
}

impl BrowserRuntime {
    pub fn new(state_dir: PathBuf) -> Self {
        Self { state_dir }
    }
}

impl fmt::Debug for BrowserRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserRuntime")
            .field("state_dir", &self.state_dir)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub full_output_ref: Option<String>,
    /// Explicit business/protocol data. Presentation text is never decoded
    /// into authoritative data by the Host dispatcher.
    pub data: Option<Value>,
    pub execution: Option<nola_host_protocol::tool_result::ExecutionReceipt>,
    pub text: String,
    pub display: String,
    pub error: bool,
    pub artifacts: Vec<nola_core::agent::host::HostArtifact>,
    pub activation: Option<nola_core::agent::host::HostCapabilityActivation>,
}

impl ToolOutcome {
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
    pub fn with_execution(mut self, execution: nola_host_protocol::tool_result::ExecutionReceipt) -> Self {
        self.execution = Some(execution);
        self
    }
    fn with_capability_activation(mut self, capability_ids: Vec<String>) -> Self {
        self.activation = (!capability_ids.is_empty()).then_some(
            nola_core::agent::host::HostCapabilityActivation {
                source: "tool_search".to_string(),
                capability_ids,
                capability_descriptors: Vec::new(),
            },
        );
        self
    }

    fn with_file_artifact(mut self, path: &Path, operation: &str) -> Self {
        if self.error {
            return self;
        }
        let path = directory_entry_path(path).unwrap_or_else(|| path.to_path_buf());
        let Ok(uri) = url::Url::from_file_path(&path) else {
            return self;
        };
        if matches!(operation, "create" | "write" | "append" | "edit" | "delete" | "move" | "copy") {
            use nola_host_protocol::tool_result::*;
            let receipt = self.execution.get_or_insert_with(|| ExecutionReceipt {
                state: ExecutionState::Completed, ..Default::default()
            });
            receipt.effects.push(ResourceEffect {
                resource_uri: uri.to_string(), operation: operation.into(), before_version: None,
                after_version: resource_content_version(&path).ok(),
            });
        }
        self.artifacts.push(nola_core::agent::host::HostArtifact {
            uri: uri.into(),
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("artifact")
                .to_string(),
            mime_type: verified_artifact_mime(&path),
            operation: Some(operation.to_string()),
        });
        self
    }

    fn with_file_artifacts<I, P>(self, paths: I, operation: &str) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        paths.into_iter().fold(self, |outcome, path| {
            outcome.with_file_artifact(path.as_ref(), operation)
        })
    }

    /// Attach the observation captured under the mutation lock, not a model
    /// declaration or a timestamp. Unknown versions remain unknown.
    fn with_file_artifact_version(
        self,
        path: &Path,
        operation: &str,
        before_version: Option<String>,
    ) -> Self {
        let mut outcome = self.with_file_artifact(path, operation);
        if let Some(effect) = outcome.execution.as_mut().and_then(|r| r.effects.last_mut()) {
            effect.before_version = before_version;
        }
        outcome
    }

    fn with_file_artifact_changes(
        mut self,
        before: &FileArtifactStates,
        after: &FileArtifactStates,
    ) -> Self {
        if self.error {
            return self;
        }
        let mut paths = before.keys().chain(after.keys()).cloned().collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        for path in paths {
            let operation = match (before.get(&path), after.get(&path)) {
                (None, Some(_)) => "create",
                (Some(_), None) => "delete",
                (Some(left), Some(right)) if left != right => "edit",
                _ => continue,
            };
            self = self.with_file_artifact(&path, operation);
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FileArtifactFingerprint {
    size: u64,
    digest: u64,
}

type FileArtifactStates = BTreeMap<PathBuf, FileArtifactFingerprint>;

fn file_artifact_fingerprint(path: &Path) -> Result<FileArtifactFingerprint, ToolOutcome> {
    let metadata = fs::symlink_metadata(path).map_err(|io_error| {
        error(format!(
            "Inspect artifact candidate {} failed: {io_error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        let version = resource_content_version(path).map_err(error)?;
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        digest.write(version.as_bytes());
        return Ok(FileArtifactFingerprint { size: metadata.len(), digest: digest.finish() });
    }
    if !metadata.is_file() {
        return Err(error(format!(
            "Artifact candidate is not a regular file: {}",
            path.display()
        )));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(|io_error| {
        error(format!(
            "Read artifact candidate {} failed: {io_error}",
            path.display()
        ))
    })?;
    let opened_metadata = file.metadata().map_err(|io_error| {
        error(format!("Inspect opened artifact failed: {io_error}"))
    })?;
    if !opened_metadata.is_file() {
        return Err(error("Artifact changed type during observation".into()));
    }
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    // Output changes include filesystem permissions, not just body changes.
    // Observe the opened file rather than classifying command strings (chmod,
    // install, build tools, etc.). Timestamps alone are not edit evidence.
    digest.write(b"file:v2:");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        digest.write_u32(opened_metadata.permissions().mode() & 0o7777);
    }
    #[cfg(not(unix))]
    digest.write_u8(u8::from(opened_metadata.permissions().readonly()));
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|io_error| {
            error(format!(
                "Read artifact candidate {} failed: {io_error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        digest.write(&buffer[..read]);
    }
    Ok(FileArtifactFingerprint {
        size: opened_metadata.len(),
        digest: digest.finish(),
    })
}

fn capture_file_artifact_states<I, P>(paths: I) -> Result<FileArtifactStates, ToolOutcome>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut states = FileArtifactStates::new();
    for path in paths {
        let path = path.as_ref();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|io_error| error(format!("Resolve artifact path failed: {io_error}")))?
                .join(path)
        };
        match fs::symlink_metadata(&absolute) {
            Ok(_) => {
                states.insert(absolute.clone(), file_artifact_fingerprint(&absolute)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(io_error) => {
                return Err(error(format!(
                    "Inspect artifact candidate {} failed: {io_error}",
                    absolute.display()
                )))
            }
        }
    }
    Ok(states)
}

fn capture_workspace_text_artifact_states(root: &Path) -> Result<FileArtifactStates, ToolOutcome> {
    let mut paths = Vec::new();
    let mut exceeded = false;
    walk_files(root, MAX_ARTIFACT_SCAN_FILES + 1, &mut |path| {
        if is_text_source(path) {
            paths.push(path.to_path_buf());
            if paths.len() > MAX_ARTIFACT_SCAN_FILES {
                exceeded = true;
                return false;
            }
        }
        true
    });
    if exceeded {
        return Err(error_with_display(
            format!(
                "Formatting scope exceeds {MAX_ARTIFACT_SCAN_FILES} text files under {}",
                root.display()
            ),
            "Formatting was not started because its changed-resource scope is too large"
                .to_string(),
        ));
    }
    capture_file_artifact_states(paths)
}

#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub id: String,
    pub name: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone)]
struct ToolSpec<'a> {
    name: &'a str,
    description: &'a str,
    parameters: Value,
    read_only: bool,
}

fn required_tool_argument_error(spec: &ToolSpec, request: &ToolRequest) -> Option<String> {
    if spec.parameters.get("additionalProperties") == Some(&Value::Bool(false)) {
        if let Some(properties) = spec.parameters.get("properties").and_then(Value::as_object) {
            let unexpected: Vec<_> = request.args.keys().filter(|key|
                key.as_str() != nola_core::agent::tool_lifecycle::TOOL_INVOCATION_CONTEXT_ARGUMENT
                    && !properties.contains_key(*key)).cloned().collect();
            if !unexpected.is_empty() {
                return Some(format!("Invalid arguments for tool '{}': unexpected field(s): {}",
                    request.name, unexpected.join(", ")));
            }
        }
    }
    let required = spec.parameters.get("required")?.as_array()?;
    let missing = required
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| match request.args.get(*name) {
            None | Some(Value::Null) => true,
            Some(Value::String(value)) if request.name == "terminal_input" && *name == "text" => {
                value.is_empty()
            }
            Some(Value::String(value)) if !matches!(*name, "content" | "new_text") => {
                value.trim().is_empty()
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Some(format!(
            "Invalid arguments for tool '{}': missing required field(s): {}",
            request.name,
            missing.join(", ")
        ));
    }
    let alternatives = spec.parameters.get("oneOf").and_then(Value::as_array);
    if let Some(alternatives) = alternatives {
        let required_alternatives = alternatives
            .iter()
            .filter_map(|alternative| alternative.get("required").and_then(Value::as_array))
            .collect::<Vec<_>>();
        let matching = required_alternatives
            .iter()
            .filter(|alternative| {
                alternative.iter().filter_map(Value::as_str).all(|name| {
                    !matches!(request.args.get(name), None | Some(Value::Null))
                })
            })
            .count();
        if required_alternatives.len() == alternatives.len() && matching != 1 {
            return Some(format!(
                "Invalid arguments for tool '{}': exactly one declared argument alternative is required",
                request.name
            ));
        }
    }
    None
}

pub fn validate_tool_request(request: &ToolRequest) -> Option<ToolOutcome> {
    validate_tool_request_with_skills(request, &SkillRuntimeContext::default())
}

#[cfg(test)]
mod interactive_terminal_schema_tests {
    use super::*;

    #[test]
    fn input_preserves_whitespace_while_identity_stays_required() {
        let spec = specs_with_skills(&SkillRuntimeContext::default()).into_iter()
            .find(|spec|spec.name == "terminal_input").unwrap();
        let mut request = ToolRequest { id:"call".into(),name:"terminal_input".into(),args:json!({
            "task_id":"task","attempt_id":"attempt","worker_generation":"generation",
            "request_id":"input","text":"\n"
        }).as_object().unwrap().clone() };
        assert!(required_tool_argument_error(&spec,&request).is_none());
        request.args.insert("text".into(),json!(" \t\n"));
        assert!(required_tool_argument_error(&spec,&request).is_none());
        request.args.insert("text".into(),json!(""));
        assert!(required_tool_argument_error(&spec,&request).is_some());
        request.args.insert("text".into(),json!("\n"));
        request.args.insert("task_id".into(),json!("  "));
        assert!(required_tool_argument_error(&spec,&request).is_some());
    }

    #[test]
    fn native_execution_catalog_matches_platform_support() {
        let available = HostCapabilityAvailability::from_captured(false,None);
        assert_eq!(available.enabled("terminal_input"),cfg!(unix));
        assert_eq!(available.enabled("language_service_query"),cfg!(unix));
        assert!(available.enabled("language_service_list"));
    }
}

fn validate_tool_request_with_skills(
    request: &ToolRequest,
    skills: &SkillRuntimeContext,
) -> Option<ToolOutcome> {
    let spec = specs_with_skills(skills)
        .into_iter()
        .find(|spec| spec.name == request.name)?;
    if let Some(message) = required_tool_argument_error(&spec, request) {
        return Some(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
            text: format!(
                "{message}\n\
tool_execution_skipped: true\n\
deterministic_failure_category: invalid_tool_arguments\n\
next_action: correct_the_reported_schema_violation; do_not_repeat_unchanged_arguments"
            ),
            display: message,
            error: true,
            artifacts: Vec::new(),
            activation: None,
        });
    }
    let command = request
        .args
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let operation_kind = request
        .args
        .get("operation_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if request.name == "bash"
        && matches!(operation_kind, "validate" | "mutate_and_validate")
        && validation_pipeline_hides_result(command)
    {
        let message = crate::i18n::interaction_text()
            .validation_pipeline_rejected
            .to_string();
        return Some(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
            text: format!(
                "{message}\n\
tool_execution_skipped: true\n\
deterministic_failure_category: truncated_validation_output\n\
next_action: run_the_validation_command_directly_and_use_its_real_exit_status"
            ),
            display: message,
            error: true,
            artifacts: Vec::new(),
            activation: None,
        });
    }
    None
}




pub fn tool_schemas() -> Vec<Value> {
    tool_schemas_with_skills(&SkillRuntimeContext::default())
}

pub fn tool_schemas_with_skills(skills: &SkillRuntimeContext) -> Vec<Value> {
    tool_schemas_with_host_metadata(false, skills)
}

pub(crate) const TOOL_NORMALIZATION_SCHEMA_KEY: &str = "x-nola-input-normalization";
pub(crate) const TOOL_READ_ONLY_SCHEMA_KEY: &str = "x-nola-read-only";

pub(crate) fn tool_schemas_for_host() -> Vec<Value> {
    tool_schemas_for_host_with_skills(&SkillRuntimeContext::default())
}

pub(crate) fn tool_schemas_for_host_with_skills(skills: &SkillRuntimeContext) -> Vec<Value> {
    tool_schemas_with_host_metadata(true, skills)
}

fn tool_schemas_with_host_metadata(
    include_host_metadata: bool,
    skills: &SkillRuntimeContext,
) -> Vec<Value> {
    tool_schemas_with_host_availability(include_host_metadata, skills, || {
        HostCapabilityAvailability::capture(skills)
    })
}

fn tool_schemas_with_host_availability(
    include_host_metadata: bool,
    skills: &SkillRuntimeContext,
    capture: impl FnOnce() -> HostCapabilityAvailability,
) -> Vec<Value> {
    let availability = capture();
    specs_with_skills(skills)
        .into_iter()
        .filter(|spec| spec.name != "skill_context")
        // Host protocol mutations are valid only inside an active Agent Host
        // execution contract. Do not leak them into the legacy generic schema
        // surface, where Core correctly rejects tools without static built-in
        // metadata. `tool_schemas_for_host()` retains them.
        .filter(|spec| include_host_metadata || spec.name != "skill_update")
        .filter(|spec| availability.enabled(spec.name))
        .map(|spec| {
            let mut parameters = spec.parameters;
            if !include_host_metadata {
                parameters
                    .as_object_mut()
                    .map(|object| object.remove(TOOL_NORMALIZATION_SCHEMA_KEY));
            }
            let mut schema = json!({
                "type": "function",
                "function": {
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": parameters,
                }
            });
            if include_host_metadata {
                schema[TOOL_READ_ONLY_SCHEMA_KEY] = Value::Bool(spec.read_only);
            }
            schema
        })
        .collect()
}



#[allow(dead_code)]
pub fn tool_summary() -> Vec<String> {
    specs()
        .into_iter()
        .map(|spec| format!("- `{}`: {}", spec.name, spec.description))
        .collect()
}

pub fn tool_is_read_only(name: &str) -> bool {
    specs()
        .into_iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.read_only)
        .unwrap_or(false)
}

pub fn tool_effect_declaration(
    name: &str,
) -> nola_core::agent::host::HostCapabilityEffectDeclaration {
    tool_effect_declaration_with_read_only(name, tool_is_read_only(name))
}

pub fn tool_effect_declaration_with_read_only(
    name: &str,
    read_only: bool,
) -> nola_core::agent::host::HostCapabilityEffectDeclaration {
    use nola_core::agent::host::{HostCapabilityEffect, HostCapabilityEffectDeclaration};

    if matches!(name, "language_service_query" | "terminal_input") {
        return HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Execute);
    }
    if name == "read_file" {
        return HostCapabilityEffectDeclaration::by_argument(
            "operation_kind",
            HostCapabilityEffect::Investigate,
            [("validate".to_string(), HostCapabilityEffect::Validate)],
        );
    }
    if name == "bash" {
        // Purpose is optional model metadata. An omitted declaration is an
        // opaque execution, not inferred inspection, mutation or validation.
        // Concrete command/resource authorization is evaluated separately.
        return HostCapabilityEffectDeclaration::by_argument(
            "operation_kind",
            HostCapabilityEffect::Execute,
            [
                ("inspect".to_string(), HostCapabilityEffect::Investigate),
                ("mutate".to_string(), HostCapabilityEffect::Mutate),
                ("validate".to_string(), HostCapabilityEffect::Validate),
                (
                    "mutate_and_validate".to_string(),
                    HostCapabilityEffect::MutateAndValidate,
                ),
                ("execute".to_string(), HostCapabilityEffect::Execute),
            ],
        );
    }
    let dry_run_default = match name {
        "notebook_edit" | "write_file" | "move_file" | "delete_file" | "edit_file"
        | "edit_file_lines" | "multi_edit_file" => Some(false),
        "format_files" | "replace_symbol_body" | "replace_in_files" | "rename_symbol"
        | "apply_patch" => Some(true),
        _ => None,
    };
    if let Some(default) = dry_run_default {
        return HostCapabilityEffectDeclaration::by_argument(
            "dry_run",
            if default {
                HostCapabilityEffect::Investigate
            } else {
                HostCapabilityEffect::Mutate
            },
            [
                ("true".to_string(), HostCapabilityEffect::Investigate),
                ("false".to_string(), HostCapabilityEffect::Mutate),
            ],
        );
    }
    if name == "agent_task_status" {
        // The Host validates the evidence attached to status=validated before
        // returning success. Preserve that typed attestation across the Host
        // boundary instead of making Core parse display text or maintain a
        // second copy of the Host's validation-evidence ledger.
        return HostCapabilityEffectDeclaration::by_argument(
            "status",
            HostCapabilityEffect::Investigate,
            [("validated".to_string(), HostCapabilityEffect::Validate)],
        );
    }
    let effect = match name {
        // These adapters verify the committed canonical record and return
        // version-bound checks. Core consumes those checks, never the tool's
        // prose, to discharge only the matching mutation obligation.
        "git_commit" | "git_worktree_create" | "git_worktree_integrate"
        | "memory_save" | "nola_orchestrate" => HostCapabilityEffect::MutateAndValidate,
        "code_diagnostics"
        | "verification_review"
        | "diff_review"
        | "document_render"
        | "document_verify"
        | "spreadsheet_verify"
        | "presentation_verify" => HostCapabilityEffect::Validate,
        // Observing a durable Task is safe to resume after a Host restart.
        // It must not create an unfinished workspace-mutation transaction.
        "list_tasks" | "read_task" | "wait_task" | "bash_background_wait" => HostCapabilityEffect::Investigate,
        "Skill"
        | "skill_invoke"
        | "skill_update"
        | "SOP"
        | "sop_invoke"
        | "sop_run_update"
        | "request_user_input"
        | "workspace_access"
        | "spawn_agent"
        | "delegate_expert"
        | "expert_exchange"
        | "send_agent_message"
        => HostCapabilityEffect::Execute,
        _ if read_only => HostCapabilityEffect::Investigate,
        _ => HostCapabilityEffect::Mutate,
    };
    HostCapabilityEffectDeclaration::fixed(effect)
}

/// Declares the durable resource boundary used to recover a built-in tool
/// effect. This is Host contract metadata, independent of prompt wording and
/// of the effect's investigate/mutate classification.
pub fn tool_transaction_boundary(
    name: &str,
) -> nola_core::agent::host::HostCapabilityTransactionBoundary {
    use nola_core::agent::host::HostCapabilityTransactionBoundary;

    match name {
        "memory_save" | "nola_orchestrate" | "delegate_expert" | "expert_exchange" | "workspace_access" | "git_worktree_create" | "git_worktree_integrate" => HostCapabilityTransactionBoundary::HostState,
        _ => HostCapabilityTransactionBoundary::Workspace,
    }
}

/// Host-owned post-mutation evidence contract. The declaration is attached to
/// the frozen capability descriptor so Core never has to inspect command text,
/// path suffixes, or user wording to decide whether validation is still due.
pub fn tool_requires_validation_after_mutation(_name: &str) -> bool {
    // Built-in atomic operations report their observed effects. A requested
    // verifier or an adopted workflow may impose an additional obligation;
    // merely choosing a file/shell tool must not invent a build/test workflow.
    false
}

/// A stricter property than read-only. These tools have no Host Turn-state
/// mutation, no interactive permission flow, and no platform bridge callback,
/// so separate invocations may execute in isolation. New tools default to
/// serial until they are explicitly audited here.
pub fn tool_supports_isolated_parallel_execution(name: &str) -> bool {
    matches!(
        name,
        "system_time"
            | "environment_probe"
            | "web_search"
            | "web_fetch"
            | "read_file"
            | "read_many_files"
            | "notebook_read"
            | "file_outline"
            | "read_symbol"
            | "repo_map"
            | "code_search"
            | "workspace_symbols"
            | "symbol_search"
            | "implementation_search"
            | "code_navigation"
            | "call_graph"
            | "code_diagnostics"
            | "failure_context"
            | "failure_triage"
            | "file_dependencies"
            | "coding_conventions"
            | "instruction_context"
    )
}

pub fn tool_names_are_read_only(names: &[String]) -> bool {
    let specs = specs();
    names.iter().all(|name| {
        specs
            .iter()
            .find(|spec| spec.name == name)
            .is_some_and(|spec| spec.read_only)
    })
}

pub(crate) fn bash_command_is_safe_read_only(
    command: &str,
    cwd: Option<&str>,
    allowed_dirs: &[PathBuf],
) -> bool {
    bash_command_path_policy(command, cwd, allowed_dirs, true) == BashCommandPathPolicy::Allowed
}

pub(crate) fn bash_command_paths_stay_in_allowed_dirs(
    command: &str,
    cwd: Option<&str>,
    allowed_dirs: &[PathBuf],
) -> bool {
    bash_command_path_policy_check(command, cwd, allowed_dirs)
        == BashCommandPathPolicy::Allowed
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashCommandPathPolicy {
    Allowed,
    UnsupportedSyntax,
    InvalidPath,
    SensitiveResource,
}

/// Pure syntax admission before showing a permission prompt. No path is opened
/// and no grant is created; the full resource check still runs at execution.
pub(crate) fn shell_permission_preflight(capability: &str, input: &Value) -> Result<(), &'static str> {
    if capability != "bash" || unrestricted_resource_policy() { return Ok(()); }
    let Some(command) = input.get("command").and_then(Value::as_str) else {
        return Err("A shell command string is required. No command was executed.");
    };
    let normalized = normalize_shell_line_continuations(command);
    if parse_shell_segments_for_path_policy(&normalized, true).is_none() {
        return Err("Command syntax cannot be safely analyzed. Use simple commands supported by the workspace checker. No command was executed and no permission decision was recorded.");
    }
    Ok(())
}

#[cfg(test)]
mod permission_preflight_tests {
    use super::*;

    #[test]
    fn syntax_admission_does_not_need_access_to_the_resource() {
        assert!(shell_permission_preflight("bash", &json!({"command": "ls /nonexistent-preflight-resource"})).is_ok());
        assert!(shell_permission_preflight("write_file", &json!({})).is_ok());
    }

    #[test]
    fn unsupported_expansion_is_rejected_before_approval() {
        assert!(shell_permission_preflight("bash", &json!({"command": "for x in a b; do cat \"$x\"; done"})).is_err());
        assert!(shell_permission_preflight("bash", &json!({})).is_err());
    }
}

pub(crate) fn bash_command_path_policy_check(
    command: &str,
    cwd: Option<&str>,
    allowed_dirs: &[PathBuf],
) -> BashCommandPathPolicy {
    bash_command_path_policy(command, cwd, allowed_dirs, false)
}

fn bash_command_path_policy(
    command: &str,
    cwd: Option<&str>,
    allowed_dirs: &[PathBuf],
    require_read_only: bool,
) -> BashCommandPathPolicy {
    let normalized_command = normalize_shell_line_continuations(command);
    let Some(segments) =
        parse_shell_segments_for_path_policy(&normalized_command, !require_read_only)
    else {
        return BashCommandPathPolicy::UnsupportedSyntax;
    };
    let cwd = cwd
        .map(PathBuf::from)
        .or_else(|| allowed_dirs.first().cloned())
        .unwrap_or_else(|| PathBuf::from("."));
    let Ok(mut current_dir) = resolve_allowed(&cwd.to_string_lossy(), allowed_dirs, true) else {
        return BashCommandPathPolicy::InvalidPath;
    };
    if !current_dir.is_dir() {
        return BashCommandPathPolicy::InvalidPath;
    }
    for tokens in segments {
        let Some(head) = tokens.first().map(String::as_str) else {
            return BashCommandPathPolicy::UnsupportedSyntax;
        };
        if head == "cd" {
            let target = match tokens.as_slice() {
                [_, target] => target,
                [_, option, target] if option == "--" => target,
                _ => return BashCommandPathPolicy::UnsupportedSyntax,
            };
            if bash_token_is_sensitive_path(target) {
                return BashCommandPathPolicy::SensitiveResource;
            }
            let candidate = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                current_dir.join(target)
            };
            let Ok(resolved) = resolve_allowed(&candidate.to_string_lossy(), allowed_dirs, true)
            else {
                return BashCommandPathPolicy::InvalidPath;
            };
            if !resolved.is_dir() {
                return BashCommandPathPolicy::InvalidPath;
            }
            current_dir = resolved;
            continue;
        }
        if require_read_only && !bash_segment_is_safe_read_only(&tokens) {
            return BashCommandPathPolicy::UnsupportedSyntax;
        }
        if !git_ignore_metadata_query(&tokens)
            && tokens.iter().skip(1).any(|token| bash_token_is_sensitive_path(token))
        {
            return BashCommandPathPolicy::SensitiveResource;
        }
        if !bash_segment_paths_stay_in_allowed_dirs(&tokens, &current_dir, allowed_dirs) {
            return BashCommandPathPolicy::InvalidPath;
        }
    }
    BashCommandPathPolicy::Allowed
}

fn normalize_shell_line_continuations(command: &str) -> String {
    let mut normalized = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    let mut single_quoted = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            single_quoted = !single_quoted;
            normalized.push(ch);
            continue;
        }
        if ch == '\\' && !single_quoted {
            if chars.next_if_eq(&'\n').is_some() {
                continue;
            }
            if chars.next_if_eq(&'\r').is_some() {
                if chars.next_if_eq(&'\n').is_some() {
                    continue;
                }
                normalized.push('\\');
                normalized.push('\r');
                continue;
            }
        }
        normalized.push(ch);
    }
    normalized
}

// native_tool_replacement_for_bash removed: bash now handles all shell commands directly
// without redirecting ls/git/head etc. to structured tools.

fn parse_shell_segments_for_path_policy(
    command: &str,
    allow_workspace_redirects: bool,
) -> Option<Vec<Vec<String>>> {
    let mut segments = Vec::new();
    let mut segment = Vec::new();
    let mut token = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else if ch == '\0' {
                return None;
            } else if active_quote == '"' && ch == '`' {
                return None;
            } else if active_quote == '"' && ch == '\\' {
                // Backslash escapes inside a quoted grep/rg expression are
                // ordinary literal syntax (for example "a\\|b"). They do
                // not perform expansion or select a filesystem path.
                let next = chars.next()?;
                if matches!(next, '\r' | '\n' | '\0') {
                    return None;
                }
                token.push(next);
            } else if active_quote == '"' && ch == '$' {
                // `$?` only exposes the previous process exit status. It does
                // not expand a path, environment secret, or command. Keep all
                // other double-quoted shell expansion rejected.
                if chars.next_if_eq(&'?').is_some() {
                    token.push('$');
                    token.push('?');
                } else {
                    return None;
                }
            } else {
                token.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '>' => {
                // Descriptor duplication (for example `2>&1`) does not access
                // the filesystem. The only path redirect allowed here is the
                // exact standard null device; every other redirect remains a
                // write outside the structured filesystem capability policy.
                let descriptor =
                    !token.is_empty() && token.chars().all(|value| value.is_ascii_digit());
                if !token.is_empty() && !descriptor {
                    segment.push(std::mem::take(&mut token));
                } else {
                    token.clear();
                }
                let append = chars.next_if_eq(&'>').is_some();
                if chars.next_if_eq(&'&').is_some() {
                    if append || !descriptor {
                        return None;
                    }
                    let Some(target) = chars.next() else {
                        return None;
                    };
                    if !target.is_ascii_digit() {
                        return None;
                    }
                    while chars.peek().is_some_and(|value| value.is_ascii_digit()) {
                        let _ = chars.next();
                    }
                } else {
                    while chars.peek().is_some_and(|value| value.is_whitespace()) {
                        let _ = chars.next();
                    }
                    let target = parse_shell_redirect_target(&mut chars)?;
                    if !allow_workspace_redirects && target != "/dev/null" {
                        return None;
                    }
                    segment.push(target);
                }
            }
            '\\' => {
                // Outside quotes, a backslash makes the following character
                // literal. This is required by ordinary read-only commands
                // such as `find . \\( ... \\)`. Preserve the escaped value in
                // the token; path validation still evaluates the resulting
                // argument against the workspace boundary.
                let next = chars.next()?;
                if matches!(next, '\r' | '\n' | '\0') {
                    return None;
                }
                token.push(next);
            }
            '`' | '$' | '<' | '\r' | '\n' | '\0' => return None,
            ch if ch.is_whitespace() => {
                if !token.is_empty() {
                    segment.push(std::mem::take(&mut token));
                }
            }
            ';' | '|' | '&' => {
                if !token.is_empty() {
                    segment.push(std::mem::take(&mut token));
                }
                if ch == '&' && chars.next_if_eq(&'&').is_none() {
                    return None;
                }
                if ch == '|' {
                    let _ = chars.next_if_eq(&'|');
                }
                if segment.is_empty() {
                    return None;
                }
                segments.push(std::mem::take(&mut segment));
            }
            _ => token.push(ch),
        }
        if segments.len() > 32 || segment.len() > 256 || token.len() > 4_000 {
            return None;
        }
    }
    if quote.is_some() {
        return None;
    }
    if !token.is_empty() {
        segment.push(token);
    }
    if segment.is_empty() {
        return None;
    }
    segments.push(segment);
    Some(segments)
}

fn parse_read_only_shell_segments(command: &str) -> Option<Vec<Vec<String>>> {
    parse_shell_segments_for_path_policy(command, false)
}

fn parse_shell_redirect_target(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<String> {
    let quote = chars
        .peek()
        .copied()
        .filter(|value| matches!(value, '\'' | '"'));
    if quote.is_some() {
        let _ = chars.next();
    }
    let mut target = String::new();
    while let Some(value) = chars.peek().copied() {
        if quote == Some(value) {
            let _ = chars.next();
            return (!target.is_empty()).then_some(target);
        }
        if quote.is_none() && (value.is_whitespace() || matches!(value, ';' | '|' | '&')) {
            break;
        }
        if value == '\0'
            || (quote != Some('\'') && matches!(value, '\\' | '`' | '$'))
            || (quote.is_none() && matches!(value, '\'' | '"' | '<' | '>'))
        {
            return None;
        }
        target.push(value);
        let _ = chars.next();
    }
    if quote.is_some() || target.is_empty() {
        None
    } else {
        Some(target)
    }
}

fn bash_segment_is_safe_read_only(tokens: &[String]) -> bool {
    let Some(head) = tokens.first().map(String::as_str) else {
        return false;
    };
    if !READ_ONLY_COMMANDS.contains(&head) || head == "cd" {
        return false;
    }
    let lowered = tokens
        .iter()
        .skip(1)
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    match head {
        "find" => !lowered.iter().any(|token| {
            matches!(
                token.as_str(),
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fls"
                    | "-fprint"
                    | "-fprint0"
            )
        }),
        "git" => {
            let Some(subcommand) = lowered.iter().find(|token| !token.starts_with('-')) else {
                return false;
            };
            matches!(
                subcommand.as_str(),
                "blame"
                    | "check-ignore"
                    | "describe"
                    | "diff"
                    | "grep"
                    | "log"
                    | "ls-files"
                    | "name-rev"
                    | "rev-parse"
                    | "shortlog"
                    | "show"
                    | "status"
            ) && !lowered.iter().any(|token| {
                token == "-c"
                    || token.starts_with("--config-env")
                    || token.starts_with("--exec-path")
                    || token.starts_with("--git-dir")
                    || token.starts_with("--work-tree")
                    || token == "--ext-diff"
                    || token == "--textconv"
            })
        }
        "rg" => !lowered.iter().any(|token| {
            token == "--pre"
                || token.starts_with("--pre=")
                || token == "--hostname-bin"
                || token.starts_with("--hostname-bin=")
        }),
        "sed" => false,
        "sort" => !lowered
            .iter()
            .any(|token| token == "-o" || token.starts_with("--output")),
        "tree" => !lowered
            .iter()
            .any(|token| token == "-o" || token.starts_with("--output")),
        "uniq" => false,
        _ => true,
    }
}

fn bash_argument_stays_in_allowed_dirs(token: &str, cwd: &Path, allowed_dirs: &[PathBuf]) -> bool {
    bash_argument_path_check(token, cwd, allowed_dirs, false)
}

fn bash_argument_path_check(token: &str, cwd: &Path, allowed_dirs: &[PathBuf], metadata_only: bool) -> bool {
    if token == "--" || token.parse::<f64>().is_ok() {
        return true;
    }
    let candidate_token = if token.starts_with('-') {
        token.split_once('=').map(|(_, value)| value).unwrap_or("")
    } else {
        token
    };
    if candidate_token.is_empty() {
        return true;
    }
    if matches!(
        candidate_token,
        "/dev/null"
            | ">/dev/null"
            | "1>/dev/null"
            | "2>/dev/null"
            | ">>/dev/null"
            | "1>>/dev/null"
            | "2>>/dev/null"
            | "</dev/null"
            | "0</dev/null"
    ) {
        return true;
    }
    if (!metadata_only && bash_token_is_sensitive_path(candidate_token)) || candidate_token.starts_with('~') {
        return false;
    }
    let looks_like_path = candidate_token.starts_with('/')
        || candidate_token.starts_with('.')
        || candidate_token.contains('/')
        || cwd.join(candidate_token).exists();
    if !looks_like_path {
        return true;
    }
    let path_prefix = candidate_token
        .find(['*', '?', '['])
        .map(|index| &candidate_token[..index])
        .unwrap_or(candidate_token);
    let path_prefix = if path_prefix.is_empty() {
        "."
    } else {
        path_prefix
    };
    let mut candidate = if Path::new(path_prefix).is_absolute() {
        PathBuf::from(path_prefix)
    } else {
        cwd.join(path_prefix)
    };
    if candidate_token.contains(['*', '?', '[']) && !candidate.is_dir() {
        candidate.pop();
    }
    while !candidate.exists() {
        if !candidate.pop() {
            return false;
        }
    }
    resolve_allowed(&candidate.to_string_lossy(), allowed_dirs, true).is_ok()
}

fn git_ignore_metadata_query(tokens: &[String]) -> bool {
    // This Git subcommand checks ignore rules for names, not file contents.
    // It does not authorize opening a secret or escaping the workspace.
    tokens.first().and_then(|program| Path::new(program).file_name()).and_then(|name| name.to_str()) == Some("git")
        && tokens.get(1).map(String::as_str) == Some("check-ignore")
}

fn bash_segment_paths_stay_in_allowed_dirs(
    tokens: &[String],
    cwd: &Path,
    allowed_dirs: &[PathBuf],
) -> bool {
    let Some(head) = tokens.first() else {
        return false;
    };
    let executable = Path::new(head)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(head);
    let inline_program_flag = match executable {
        "node" | "ruby" | "perl" => Some("-e"),
        value if value == "python" || value.starts_with("python3") => Some("-c"),
        _ => None,
    };
    let mut index = 1;
    while index < tokens.len() {
        let token = &tokens[index];
        if inline_program_flag == Some(token.as_str()) {
            let Some(program) = tokens.get(index + 1) else {
                return false;
            };
            if !bash_inline_program_paths_stay_in_allowed_dirs(program, cwd, allowed_dirs) {
                return false;
            }
            index += 2;
            continue;
        }
        if !bash_argument_path_check(token, cwd, allowed_dirs, git_ignore_metadata_query(tokens)) {
            return false;
        }
        index += 1;
    }
    true
}

fn bash_inline_program_paths_stay_in_allowed_dirs(
    program: &str,
    cwd: &Path,
    allowed_dirs: &[PathBuf],
) -> bool {
    let mut chars = program.chars().peekable();
    while let Some(ch) = chars.next() {
        if !matches!(ch, '\'' | '"') {
            continue;
        }
        let quote = ch;
        let mut literal = String::new();
        let mut escaped = false;
        for value in chars.by_ref() {
            if escaped {
                literal.push(value);
                escaped = false;
                continue;
            }
            if value == '\\' {
                escaped = true;
                literal.push(value);
                continue;
            }
            if value == quote {
                break;
            }
            literal.push(value);
        }
        let candidate = literal.trim();
        if candidate.is_empty()
            || candidate.contains("://")
            || candidate.starts_with(".//")
            || candidate.starts_with("{//")
        {
            continue;
        }
        if !bash_argument_stays_in_allowed_dirs(candidate, cwd, allowed_dirs) {
            return false;
        }
    }
    true
}

fn bash_token_is_sensitive_path(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.split(['/', '\\']).any(|component| {
        component == ".env"
            || (component.starts_with(".env.")
                && !matches!(component, ".env.example" | ".env.sample" | ".env.template"))
            || matches!(
                component,
                ".ssh" | ".gnupg" | ".aws" | ".azure" | ".kube" | "keychains"
                    | ".npmrc"
                    | ".pypirc"
                    | "credentials"
                    | "credentials.json"
                    | "id_dsa"
                    | "id_ed25519"
                    | "id_rsa"
                    | "secrets"
                    | "secrets.json"
            )
            || component.ends_with(".key")
            || component.ends_with(".p12")
            || component.ends_with(".pem")
            || component.ends_with(".pfx")
    })
}


fn permission_caches() -> &'static Mutex<HashMap<String, PermissionCache>> {
    SESSION_PERMISSION_CACHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_permission_scope_key() -> String {
    CURRENT_PERMISSION_SCOPE
        .with(|scope| scope.borrow().current().cloned())
        .unwrap_or_else(|| "unscoped".to_string())
}

fn permission_scope_is_active() -> bool {
    CURRENT_PERMISSION_SCOPE.with(|scope| scope.borrow().current().is_some())
}

fn with_permission_cache<R>(action: impl FnOnce(&PermissionCache) -> R) -> Option<R> {
    let key = current_permission_scope_key();
    let caches = permission_caches().lock().ok()?;
    let cache = caches.get(&key)?;
    Some(action(cache))
}

fn with_permission_cache_mut<R>(action: impl FnOnce(&mut PermissionCache) -> R) -> Option<R> {
    let key = current_permission_scope_key();
    let mut caches = permission_caches().lock().ok()?;
    let cache = caches.entry(key).or_default();
    Some(action(cache))
}

fn push_unique_string(items: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !items.iter().any(|item| item == value) {
        items.push(value.to_string());
    }
}

fn push_unique_path(items: &mut Vec<PathBuf>, value: PathBuf) {
    if !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

pub fn configure_session_permission_config(path: Option<PathBuf>) {
    with_permission_cache_mut(|cache| {
        cache.config_path = path.clone();
        cache.allowed_dirs.clear();
    });
    if let Some(path) = path {
        load_session_allowed_dirs_from_config(&path);
    }
}

pub fn configure_session_permission_scope(session_id: Option<String>) {
    let scope = session_id.map(explicit_or_session_permission_scope);
    with_permission_cache_mut(|cache| {
        cache.scope = scope.clone();
        cache.persistent_grants = true;
        cache.allowed_dirs.clear();
        cache.tool_rules.clear();
        cache.bash_prefixes.clear();
        cache.network_rules.clear();
        cache.agent_rules.clear();
    });
    let Some(scope) = scope else {
        return;
    };
    if let Ok(grants) = crate::permission_grants::permission_grants_for_scope(&scope) {
        for grant in grants.into_iter().filter(|grant| {
            grant.decision == nola_application::PermissionGrantDecision::Allow
        }) {
            match grant.capability_pattern.as_str() {
                "cli::file::allowed_dir" => {
                    if let Ok(dir) = normalize_allowed_dir(&grant.resource_pattern) {
                        add_session_allowed_dir(dir);
                    }
                }
                "cli::tool::allow_rule" => add_session_tool_allow_rule(grant.resource_pattern),
                "cli::bash::allow_prefix" => {
                    add_session_bash_allow_prefix(grant.resource_pattern)
                }
                "cli::network::allow_rule" => {
                    add_session_network_allow_rule(grant.resource_pattern)
                }
                "cli::agent::allow_rule" => add_session_agent_allow_rule(grant.resource_pattern),
                _ => {}
            }
        }
    }
}

pub fn configure_ephemeral_session_permission_scope(session_id: Option<String>) {
    let scope = session_id.map(explicit_or_session_permission_scope);
    with_permission_cache_mut(|cache| {
        cache.scope = scope;
        cache.persistent_grants = false;
        cache.allowed_dirs.clear();
        cache.tool_rules.clear();
        cache.bash_prefixes.clear();
        cache.network_rules.clear();
        cache.agent_rules.clear();
    });
}

fn explicit_or_session_permission_scope(id: String) -> String {
    if [
        "session:",
        "conversation:",
        "host:",
        "user:",
        "global",
        "web:",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
    {
        id
    } else {
        format!("session:{id}")
    }
}

#[allow(dead_code)]
pub fn load_session_permission_grants(
    tool_name: &str,
    behavior: &str,
    action: &str,
) -> Vec<String> {
    if !with_permission_cache(|cache| cache.persistent_grants).unwrap_or(false) {
        return Vec::new();
    }
    current_session_permission_scope()
        .and_then(|scope| {
            crate::permission_grants::permission_grants(
                &scope, tool_name, behavior, action,
            )
            .ok()
        })
        .unwrap_or_default()
}

pub fn write_session_permission_grant(
    tool_name: &str,
    behavior: &str,
    pattern: &str,
    action: &str,
) -> Result<(), String> {
    let Some(scope) = current_session_permission_scope() else {
        return Ok(());
    };
    if !with_permission_cache(|cache| cache.persistent_grants).unwrap_or(false) {
        return Ok(());
    }
    crate::permission_grants::add_permission_grant(
        &scope, tool_name, behavior, pattern, action,
    )
}

fn has_session_permission_scope() -> bool {
    current_session_permission_scope().is_some()
}

fn load_session_allowed_dirs_from_config(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(items) = value.get("allowed_dirs").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        if let Some(raw) = item.as_str() {
            if let Ok(dir) = normalize_allowed_dir(raw) {
                add_session_allowed_dir(dir);
            }
        }
    }
}

fn automatic_workspace_checkpoint(
    request: &ToolRequest,
    ctx: &ToolContext,
    effect: nola_core::agent::host::HostCapabilityEffect,
    transaction_boundary: nola_core::agent::host::HostCapabilityTransactionBoundary,
) -> Result<Option<String>, ToolOutcome> {
    use nola_core::agent::host::HostCapabilityEffect;

    if !transaction_boundary.requires_workspace_checkpoint()
        || !matches!(
        effect,
        HostCapabilityEffect::Mutate | HostCapabilityEffect::MutateAndValidate
    )
    {
        return Ok(None);
    }
    if request.name == "bash_background_output_settlement" {
        return Ok(None);
    }
    if request
        .args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let Some(root) = ctx.allowed_dirs.first() else {
        return Ok(None);
    };
    if request.name == "bash" && request.args.contains_key("output_artifacts") {
        // A Git snapshot of the default workspace cannot recover outputs in a
        // different authorized root. Declared shell effects use a bounded,
        // exact-resource preimage, independent of cwd and repository layout.
        return automatic_file_preimage_checkpoint(request, ctx, root);
    }
    let git_probe = match run_git(root, &["rev-parse", "--show-toplevel"]) {
        Ok(output) => output,
        Err(_) => return automatic_file_preimage_checkpoint(request, ctx, root),
    };
    if !git_probe.status.success() {
        return automatic_file_preimage_checkpoint(request, ctx, root);
    }
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.to_string_lossy().into_owned()),
    );
    args.insert("action".to_string(), Value::String("create".to_string()));
    args.insert(
        "label".to_string(),
        Value::String(format!("before {} {}", request.name, request.id)),
    );
    let checkpoint = workspace_checkpoint(&args, ctx);
    if checkpoint.error {
        return Err(error_with_display(
            format!(
                "Automatic workspace checkpoint failed before {}: {}",
                request.name, checkpoint.text
            ),
            "Workspace checkpoint failed; the side effect was not executed".to_string(),
        ));
    }
    let id = checkpoint
        .text
        .lines()
        .find_map(|line| line.trim().strip_prefix("checkpoint_id:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            error_with_display(
                "Automatic workspace checkpoint did not return a checkpoint id".to_string(),
                "Workspace checkpoint failed; the side effect was not executed".to_string(),
            )
        })?;
    Ok(Some(id))
}

const MAX_PREIMAGE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PREIMAGE_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

fn automatic_file_preimage_checkpoint(
    request: &ToolRequest,
    ctx: &ToolContext,
    root: &Path,
) -> Result<Option<String>, ToolOutcome> {
    let raw_paths = match request.name.as_str() {
        "write_file"
        | "append_file"
        | "delete_file"
        | "replace_symbol_body"
        | "edit_file"
        | "edit_file_lines"
        | "multi_edit_file" => request
            .args
            .get("path")
            .and_then(Value::as_str)
            .map(|path| vec![path.to_string()])
            .unwrap_or_default(),
        "move_file" => {
            let mut paths = ["source", "destination"].into_iter()
                .filter_map(|key| request.args.get(key).and_then(Value::as_str)).map(str::to_string).collect::<Vec<_>>();
            if request.args.get("update_imports").and_then(Value::as_bool) == Some(true) {
                let input = Value::Object(request.args.clone());
                let plan = move_import_plan(ctx, &input).map_err(error)?;
                permission_resolution::validate_move_import_plan(ctx, &input, &plan).map_err(permission_resource_execution_failure)?;
                paths.extend(plan.updates.iter().map(|update| update.path.to_string_lossy().into_owned()));
            }
            paths
        },
        "format_files" => {
            let mut paths = Vec::new();
            walk_files(root, MAX_ARTIFACT_SCAN_FILES + 1, &mut |path| {
                if is_text_source(path) {
                    paths.push(path.to_string_lossy().into_owned());
                }
                paths.len() <= MAX_ARTIFACT_SCAN_FILES
            });
            if paths.len() > MAX_ARTIFACT_SCAN_FILES {
                return Err(error_with_display(
                    format!(
                        "Formatting recovery scope exceeds {MAX_ARTIFACT_SCAN_FILES} text files under {}",
                        root.display()
                    ),
                    "File recovery checkpoint is too large; formatting was not executed"
                        .to_string(),
                ));
            }
            paths
        }
        "bash" => {
            let cwd = request.args.get("cwd").and_then(Value::as_str)
                .map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
            declared_bash_outputs(&request.args, &cwd, &ctx.allowed_dirs)?
                .into_iter().map(|output| output.path.to_string_lossy().into_owned()).collect()
        }
        _ => Vec::new(),
    };
    if raw_paths.is_empty() {
        return Ok(None);
    }

    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let resource_scoped = request.name == "bash";
    let resource_roots = effective_allowed_dirs(&ctx.allowed_dirs);
    let mut targets = Vec::new();
    for raw in raw_paths {
        let target = resolve_allowed(&raw, &ctx.allowed_dirs, false)?;
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    let store = NolaStore::open_default().map_err(|error| {
        error_with_display(
            format!("Open file preimage store failed: {error}"),
            "File recovery checkpoint failed; the side effect was not executed".to_string(),
        )
    })?;
    let checkpoint_id = format!("preimage-{}", uuid::Uuid::new_v4());
    let base = store
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("workspace-preimages");
    let temporary = base.join(format!(".{checkpoint_id}.tmp"));
    let destination = base.join(&checkpoint_id);
    fs::create_dir_all(&temporary).map_err(|error| {
        error_with_display(
            format!("Create file preimage directory failed: {error}"),
            "File recovery checkpoint failed; the side effect was not executed".to_string(),
        )
    })?;

    let mut total_bytes = 0_u64;
    let mut entries = Vec::new();
    let mut absent = Vec::new();
    let checkpoint_result = (|| -> Result<(), ToolOutcome> {
        for (index, target) in targets.iter().enumerate() {
            let resource_root = if resource_scoped {
                resource_roots.iter().filter(|candidate| target.starts_with(candidate))
                    .max_by_key(|candidate| candidate.components().count())
                    .ok_or_else(|| error(format!("cannot resolve filesystem root for {}", target.display())))?
            } else {
                &root
            };
            let relative = target
                .strip_prefix(resource_root)
                .map(path_slash)
                .unwrap_or_else(|_| target.to_string_lossy().into_owned());
            match fs::symlink_metadata(target) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(error_with_display(
                        format!("File preimage refuses symbolic link: {}", target.display()),
                        "File recovery checkpoint failed; the side effect was not executed"
                            .to_string(),
                    ));
                }
                Ok(metadata) if !metadata.is_file() => {
                    return Err(error_with_display(
                        format!("File preimage supports files only: {}", target.display()),
                        "File recovery checkpoint failed; the side effect was not executed"
                            .to_string(),
                    ));
                }
                Ok(metadata) => {
                    let size = metadata.len();
                    total_bytes = total_bytes.saturating_add(size);
                    if size > MAX_PREIMAGE_FILE_BYTES || total_bytes > MAX_PREIMAGE_TOTAL_BYTES {
                        return Err(error_with_display(
                            format!(
                                "File preimage exceeds its bounded recovery budget: {} ({} bytes, {} bytes total)",
                                target.display(), size, total_bytes
                            ),
                            "File recovery checkpoint is too large; the side effect was not executed"
                                .to_string(),
                        ));
                    }
                    let backup_name = format!("{index}.bin");
                    fs::copy(target, temporary.join(&backup_name)).map_err(|error| {
                        error_with_display(
                            format!(
                                "Copy file preimage failed for {}: {error}",
                                target.display()
                            ),
                            "File recovery checkpoint failed; the side effect was not executed"
                                .to_string(),
                        )
                    })?;
                    entries.push(json!({
                        "path": target.to_string_lossy(),
                        "resource_root": resource_scoped.then(|| resource_root.to_string_lossy()),
                        "relative_path": relative,
                        "existed": true,
                        "size": size,
                        "backup_file": backup_name,
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    absent.push(relative.clone());
                    entries.push(json!({
                        "path": target.to_string_lossy(),
                        "resource_root": resource_scoped.then(|| resource_root.to_string_lossy()),
                        "relative_path": relative,
                        "existed": false,
                        "size": 0,
                        "backup_file": Value::Null,
                    }));
                }
                Err(error) => {
                    return Err(error_with_display(
                        format!(
                            "Inspect file preimage target failed for {}: {error}",
                            target.display()
                        ),
                        "File recovery checkpoint failed; the side effect was not executed"
                            .to_string(),
                    ));
                }
            }
        }
        let manifest = json!({
            "version": 1,
            "resource_scoped": resource_scoped,
            "kind": "file_preimage",
            "tool_call_id": request.id,
            "capability_id": request.name,
            "root": root.to_string_lossy(),
            "total_bytes": total_bytes,
            "entries": entries,
        });
        fs::write(
            temporary.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|encode_error| error(encode_error.to_string()))?,
        )
        .map_err(|io_error| error(format!("Write file preimage manifest failed: {io_error}")))?;
        fs::create_dir_all(&base)
            .map_err(|io_error| error(format!("Create preimage store failed: {io_error}")))?;
        fs::rename(&temporary, &destination).map_err(|io_error| {
            error(format!("Commit file preimage directory failed: {io_error}"))
        })?;
        store
            .save_workspace_checkpoint(&DbWorkspaceCheckpoint {
                id: checkpoint_id.clone(),
                root: root.to_string_lossy().into_owned(),
                // This persisted field is the workspace ownership boundary.
                // A file preimage has no Git repository, so retain the
                // canonical workspace root instead of weakening the Store
                // invariant with an empty owner.
                git_root: root.to_string_lossy().into_owned(),
                revision: destination.to_string_lossy().into_owned(),
                source: "file_preimage".to_string(),
                label: format!("before {} {}", request.name, request.id),
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0),
                status: manifest.to_string(),
                untracked: absent,
            })
            .map_err(error)?;
        let root_text = root.to_string_lossy().into_owned();
        if let Ok(records) = store.list_workspace_checkpoints(&root_text) {
            let excess = records.len().saturating_sub(MAX_CHECKPOINTS);
            for stale in records.into_iter().take(excess) {
                if store.delete_workspace_checkpoint(&stale.id).is_ok()
                    && stale.source == "file_preimage"
                {
                    let stale_path = PathBuf::from(stale.revision);
                    if stale_path.parent() == Some(base.as_path()) {
                        let _ = fs::remove_dir_all(stale_path);
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(outcome) = checkpoint_result {
        let _ = fs::remove_dir_all(&temporary);
        let _ = fs::remove_dir_all(&destination);
        return Err(outcome);
    }
    Ok(Some(checkpoint_id))
}

fn file_preimage_relative_path(raw: &str) -> Result<PathBuf, ToolOutcome> {
    let path = Path::new(raw);
    if raw.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(error_with_display(
            format!("Invalid file preimage relative path: {raw}"),
            "File recovery checkpoint is invalid; no files were changed".to_string(),
        ));
    }
    Ok(path.to_path_buf())
}

fn file_preimage_checkpoint_action(
    checkpoint: &WorkspaceCheckpoint,
    root: &Path,
    ctx: &ToolContext,
    action: &str,
    max_diff_chars: usize,
    dry_run: bool,
) -> ToolOutcome {
    let manifest: Value = match serde_json::from_str(&checkpoint.status) {
        Ok(value) => value,
        Err(error) => {
            return error_with_display(
                format!("Decode file preimage manifest failed: {error}"),
                "File recovery checkpoint is invalid; no files were changed".to_string(),
            )
        }
    };
    let version = manifest.get("version").and_then(Value::as_u64);
    if version != Some(1)
        || manifest.get("kind").and_then(Value::as_str) != Some("file_preimage")
    {
        return error_with_display(
            "Unsupported file preimage manifest".to_string(),
            "File recovery checkpoint is invalid; no files were changed".to_string(),
        );
    }
    let Some(resource_scoped) = manifest.get("resource_scoped").and_then(Value::as_bool) else {
        return error("File preimage resource scope is missing".into());
    };
    let manifest_root = manifest.get("root").and_then(Value::as_str).unwrap_or("");
    if Path::new(manifest_root) != root || checkpoint.root != root {
        return error_with_display(
            format!(
                "File preimage root mismatch: manifest={manifest_root} checkpoint={} requested={}",
                checkpoint.root.display(),
                root.display()
            ),
            "File recovery checkpoint belongs to another workspace; no files were changed"
                .to_string(),
        );
    }
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(store_error) => {
            return error(format!("Open file preimage store failed: {store_error}"))
        }
    };
    let managed_base = store
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("workspace-preimages");
    let managed_base = match managed_base.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return error_with_display(
                format!("Resolve file preimage store failed: {error}"),
                "File recovery checkpoint is unavailable; no files were changed".to_string(),
            )
        }
    };
    let backup_root = match Path::new(&checkpoint.revision).canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return error_with_display(
                format!("Resolve file preimage backup failed: {error}"),
                "File recovery checkpoint is unavailable; no files were changed".to_string(),
            )
        }
    };
    if backup_root.parent() != Some(managed_base.as_path())
        || backup_root.file_name().and_then(|value| value.to_str()) != Some(checkpoint.id.as_str())
    {
        return error_with_display(
            format!(
                "File preimage backup is outside the managed store: {}",
                backup_root.display()
            ),
            "File recovery checkpoint is invalid; no files were changed".to_string(),
        );
    }
    let Some(entries) = manifest.get("entries").and_then(Value::as_array) else {
        return error_with_display(
            "File preimage manifest has no entries".to_string(),
            "File recovery checkpoint is invalid; no files were changed".to_string(),
        );
    };
    if entries.is_empty() || (resource_scoped && entries.len() > 100) {
        return error_with_display(
            "File preimage manifest is empty".to_string(),
            "File recovery checkpoint is invalid; no files were changed".to_string(),
        );
    }

    struct RestoreEntry {
        relative: String,
        target: PathBuf,
        backup: Option<PathBuf>,
        existed: bool,
        state: &'static str,
    }
    let mut restore_entries = Vec::with_capacity(entries.len());
    let mut total_backup_bytes = 0_u64;
    for entry in entries {
        let Some(relative_raw) = entry.get("relative_path").and_then(Value::as_str) else {
            return error_with_display(
                "File preimage entry has no relative_path".to_string(),
                "File recovery checkpoint is invalid; no files were changed".to_string(),
            );
        };
        let relative_path = match file_preimage_relative_path(relative_raw) {
            Ok(path) => path,
            Err(outcome) => return outcome,
        };
        // Resource-scoped checkpoints store each resource's authorized root. The
        // checkpoint owner/root is bookkeeping, not a grant for its entries.
        let resource_root = if resource_scoped {
            let Some(raw) = entry.get("resource_root").and_then(Value::as_str) else {
                return error("File preimage resource root is missing".to_string());
            };
            let recorded = PathBuf::from(raw);
            if !recorded.is_absolute()
                || recorded.canonicalize().ok().as_ref() != Some(&recorded)
            {
                return error_with_display(
                    "File preimage resource root changed or is unavailable".to_string(),
                    "File recovery checkpoint cannot be applied safely; no files were changed".to_string(),
                );
            }
            recorded
        } else {
            root.to_path_buf()
        };
        let target = resource_root.join(&relative_path);
        if resource_scoped {
            if entry.get("path").and_then(Value::as_str).map(Path::new)
                != Some(target.as_path())
            {
                return error_with_display(
                    "File preimage resource identity does not match its recorded path".to_string(),
                    "File recovery checkpoint is invalid; no files were changed".to_string(),
                );
            }
            match resolve_allowed(&target.to_string_lossy(), &ctx.allowed_dirs, false) {
                Ok(resolved) if resolved == target => {}
                Ok(_) => return error_with_display(
                    "File preimage resource was redirected by a symbolic link".to_string(),
                    "File recovery checkpoint cannot be applied safely; no files were changed".to_string(),
                ),
                Err(outcome) => return outcome,
            }
        }
        let parent = match target.parent().and_then(|path| path.canonicalize().ok()) {
            Some(parent) if parent.starts_with(&resource_root)
                && (!resource_scoped || Some(parent.as_path()) == target.parent()) => parent,
            _ => {
                return error_with_display(
                    format!(
                        "File preimage target parent is unavailable or outside the workspace: {}",
                        target.display()
                    ),
                    "File recovery checkpoint cannot be applied safely; no files were changed"
                        .to_string(),
                )
            }
        };
        let target = parent.join(
            target
                .file_name()
                .expect("validated relative file path has a file name"),
        );
        if restore_entries.iter().any(|existing: &RestoreEntry| existing.target == target) {
            return error_with_display(
                "File preimage contains duplicate resource identities".to_string(),
                "File recovery checkpoint is invalid; no files were changed".to_string(),
            );
        }
        let existed = entry
            .get("existed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let backup = if existed {
            let Some(backup_name) = entry.get("backup_file").and_then(Value::as_str) else {
                return error_with_display(
                    format!("File preimage entry has no backup: {relative_raw}"),
                    "File recovery checkpoint is invalid; no files were changed".to_string(),
                );
            };
            let backup_relative = match file_preimage_relative_path(backup_name) {
                Ok(path) if path.components().count() == 1 => path,
                _ => {
                    return error_with_display(
                        format!("Invalid file preimage backup name: {backup_name}"),
                        "File recovery checkpoint is invalid; no files were changed".to_string(),
                    )
                }
            };
            let backup = backup_root.join(backup_relative);
            let metadata = match fs::symlink_metadata(&backup) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata
                }
                _ => {
                    return error_with_display(
                        format!("File preimage backup is unavailable: {}", backup.display()),
                        "File recovery checkpoint is unavailable; no files were changed"
                            .to_string(),
                    )
                }
            };
            total_backup_bytes = total_backup_bytes.saturating_add(metadata.len());
            if metadata.len() > MAX_PREIMAGE_FILE_BYTES
                || total_backup_bytes > MAX_PREIMAGE_TOTAL_BYTES
            {
                return error_with_display(
                    format!(
                        "File preimage restore exceeds its bounded recovery budget: {}",
                        backup.display()
                    ),
                    "File recovery checkpoint is too large; no files were changed".to_string(),
                );
            }
            Some(backup)
        } else {
            None
        };
        let state = match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return error_with_display(
                    format!(
                        "File preimage refuses symbolic link target: {}",
                        target.display()
                    ),
                    "File recovery checkpoint cannot be applied safely; no files were changed"
                        .to_string(),
                )
            }
            Ok(metadata) if !metadata.is_file() => {
                return error_with_display(
                    format!("File preimage target is not a file: {}", target.display()),
                    "File recovery checkpoint cannot be applied safely; no files were changed"
                        .to_string(),
                )
            }
            Ok(metadata) if existed => {
                let unchanged = backup
                    .as_ref()
                    .filter(|_| metadata.len() <= MAX_PREIMAGE_FILE_BYTES)
                    .and_then(|backup| {
                        let backup_metadata = fs::metadata(backup).ok()?;
                        if backup_metadata.len() != metadata.len() {
                            return Some(false);
                        }
                        Some(fs::read(backup).ok()? == fs::read(&target).ok()?)
                    })
                    .unwrap_or(false);
                if unchanged {
                    "unchanged"
                } else {
                    "modified"
                }
            }
            Ok(_) => "created after checkpoint",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && existed => "missing",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent",
            Err(error) => {
                return error_with_display(
                    format!(
                        "Inspect file preimage target failed for {}: {error}",
                        target.display()
                    ),
                    "File recovery checkpoint cannot be applied safely; no files were changed"
                        .to_string(),
                )
            }
        };
        restore_entries.push(RestoreEntry {
            relative: if resource_scoped { target.to_string_lossy().into_owned() }
                else { relative_raw.to_string() },
            target,
            backup,
            existed,
            state,
        });
    }

    let mut output = vec![
        if action == "restore" {
            if dry_run {
                "# File recovery restore preview".to_string()
            } else {
                "# File recovery checkpoint restored".to_string()
            }
        } else {
            "# File recovery checkpoint diff".to_string()
        },
        format!("checkpoint_id: {}", checkpoint.id),
        format!("root: {}", root.display()),
        format!("files: {}", restore_entries.len()),
        format!("backup_bytes: {total_backup_bytes}"),
        String::new(),
        "## Files".to_string(),
    ];
    for entry in &restore_entries {
        output.push(format!(
            "- {} baseline={} current={}",
            entry.relative,
            if entry.existed { "file" } else { "absent" },
            entry.state
        ));
    }
    let joined = output.join("\n");
    let mut output = joined.chars().take(max_diff_chars).collect::<String>();
    if output.chars().count() < joined.chars().count() {
        output.push_str("\n...[file list truncated]");
    }
    if action != "restore" || dry_run {
        if action == "restore" {
            output.push_str(
                "\n\nNo files changed. Call again with dry_run=false to apply this recovery checkpoint.",
            );
        }
        return ok(
            output,
            if action == "restore" {
                format!("File recovery restore preview {}", checkpoint.id)
            } else {
                format!("File recovery checkpoint diff {}", checkpoint.id)
            },
        );
    }

    let before = match capture_file_artifact_states(
        restore_entries.iter().map(|entry| entry.target.as_path()),
    ) {
        Ok(states) => states,
        Err(outcome) => return outcome,
    };

    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in &restore_entries {
        let Some(backup) = entry.backup.as_ref() else {
            continue;
        };
        let temporary = entry.target.with_file_name(format!(
            ".nola-restore-{}-{}",
            checkpoint.id,
            uuid::Uuid::new_v4()
        ));
        if let Err(error) = fs::copy(backup, &temporary) {
            for (path, _) in &staged {
                let _ = fs::remove_file(path);
            }
            return error_with_display(
                format!(
                    "Stage file preimage restore failed for {}: {error}",
                    entry.target.display()
                ),
                "File recovery checkpoint could not be staged; no files were changed".to_string(),
            );
        }
        staged.push((temporary, entry.target.clone()));
    }
    for (temporary, target) in &staged {
        if let Err(error) = fs::rename(temporary, target) {
            for (remaining, _) in &staged {
                let _ = fs::remove_file(remaining);
            }
            return error_with_display(
                format!("Apply file preimage restore failed for {}: {error}", target.display()),
                "File recovery checkpoint was only partially applied; retry the same checkpoint to finish recovery"
                    .to_string(),
            );
        }
    }
    let mut removed = 0_usize;
    for entry in &restore_entries {
        if !entry.existed && entry.target.is_file() {
            if let Err(error) = fs::remove_file(&entry.target) {
                return error_with_display(
                    format!(
                        "Remove post-checkpoint file failed for {}: {error}",
                        entry.target.display()
                    ),
                    "File recovery checkpoint was only partially applied; retry the same checkpoint to finish recovery"
                        .to_string(),
                );
            }
            removed += 1;
        }
    }
    output.push_str(&format!(
        "\n\nrestored_files: {}\nremoved_post_checkpoint_files: {removed}",
        staged.len()
    ));
    let after = match capture_file_artifact_states(
        restore_entries.iter().map(|entry| entry.target.as_path()),
    ) {
        Ok(states) => states,
        Err(outcome) => return outcome,
    };
    ok(
        output,
        format!("File recovery checkpoint restored {}", checkpoint.id),
    )
    .with_file_artifact_changes(&before, &after)
}

fn tool_outcome_transaction_value(outcome: &ToolOutcome) -> Value {
    json!({
        "full_output_ref": outcome.full_output_ref,
        "data": outcome.data,
        "execution": outcome.execution,
        "text": outcome.text,
        "display": outcome.display,
        "error": outcome.error,
        "artifacts": outcome.artifacts,
        "activation": outcome.activation,
    })
}

#[test]
fn transaction_replay_retains_code_provenance_and_execution_receipt() {
    let outcome = code_evidence::annotate("rename_symbol",
        ok("Renamed textual matches".into(), "Renamed".into())
            .with_execution(nola_host_protocol::tool_result::ExecutionReceipt {
                state: nola_host_protocol::tool_result::ExecutionState::Completed,
                ..Default::default()
            }));
    let saved = tool_outcome_transaction_value(&outcome);
    // Exercise the serialized storage boundary, not just a borrowed Value.
    let persisted: Value = serde_json::from_str(&saved.to_string()).unwrap();
    let replay = tool_outcome_from_transaction(&persisted).unwrap();
    assert_eq!(replay.text, outcome.text);
    assert_eq!(replay.data, outcome.data);
    assert_eq!(replay.execution, outcome.execution);
    assert_eq!(replay.data.unwrap()["code_evidence"]["semantic_binding_verified"], false);
}


fn tool_outcome_from_transaction(value: &Value) -> Option<ToolOutcome> {
    Some(ToolOutcome {
        full_output_ref: value.get("full_output_ref").and_then(Value::as_str).map(str::to_string),
        data: value.get("data").filter(|v| !v.is_null()).cloned(),
        execution: value.get("execution").filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone())).transpose().ok()?,
        text: value.get("text")?.as_str()?.to_string(),
        display: value.get("display")?.as_str()?.to_string(),
        error: value.get("error")?.as_bool()?,
        artifacts: value
            .get("artifacts")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .ok()?
            .unwrap_or_default(),
        activation: value
            .get("activation")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .ok()?,
    })
}

fn prepare_tool_work_step_transaction(
    request: &ToolRequest,
    ctx: &ToolContext,
    spec: &ToolSpec,
) -> Result<Option<nola_application::WorkStepTransaction>, ToolOutcome> {
    let effect = tool_effect_declaration_with_read_only(&request.name, spec.read_only)
        .resolve(&Value::Object(request.args.clone()));
    if spec.read_only {
        return Ok(None);
    }
    prepare_effect_work_step_transaction(
        request,
        ctx,
        effect,
        tool_transaction_boundary(&request.name),
    )
}

/// Starts the durable recovery boundary for a concrete Host capability call.
/// The effect is resolved from the frozen capability descriptor by Core/Host;
/// callers must never derive it from the user's wording or a display name.
pub(crate) fn prepare_effect_work_step_transaction(
    request: &ToolRequest,
    ctx: &ToolContext,
    effect: nola_core::agent::host::HostCapabilityEffect,
    transaction_boundary: nola_core::agent::host::HostCapabilityTransactionBoundary,
) -> Result<Option<nola_application::WorkStepTransaction>, ToolOutcome> {
    use nola_core::agent::host::HostCapabilityEffect;

    if matches!(
        effect,
        HostCapabilityEffect::Investigate | HostCapabilityEffect::Validate
    ) {
        return Ok(None);
    }
    let root = ctx
        .allowed_dirs
        .first()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let root_text = root.to_string_lossy().into_owned();
    let work_id = ctx
        .permission_scope
        .owner_session_id()
        .unwrap_or_else(|| format!("workspace:{root_text}"));
    let step_id = ctx
        .plan_state
        .current_todos()
        .ok()
        .and_then(|todos| {
            todos
                .into_iter()
                .find(|todo| todo.status == "in_progress")
                .map(|todo| todo.id)
        })
        .unwrap_or_else(|| request.id.clone());
    let effect_kind = serde_json::to_value(effect)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "execute".to_string());
    let transaction_id = format!("work-step:{work_id}:{}", request.id);
    let prepared = crate::work_step_port::prepare_work_step_transaction(
            &transaction_id,
            &work_id,
            &step_id,
            &request.id,
            &root_text,
            &effect_kind,
            None,
            json!({"capability_id": request.name, "arguments": request.args}),
        )
        .map_err(|error| {
            error_with_display(
                format!("Prepare work step transaction failed: {error}"),
                "The side effect was not executed because its recovery record could not be created"
                    .to_string(),
            )
        })?;
    match prepared.transaction.phase {
        nola_application::WorkStepTransactionPhase::Committed => prepared
            .transaction
            .result
            .as_ref()
            .and_then(tool_outcome_from_transaction)
            .map(Err)
            .unwrap_or_else(|| {
                Err(error_with_display(
                    "Committed work step transaction has no replayable result".to_string(),
                    "The prior tool result could not be replayed".to_string(),
                ))
            }),
        nola_application::WorkStepTransactionPhase::Failed => Err(error_with_display(
            prepared
                .transaction
                .error
                .unwrap_or_else(|| "the prior work step transaction failed".to_string()),
            "The prior tool execution failed; it was not repeated".to_string(),
        )),
        nola_application::WorkStepTransactionPhase::Checkpointed
        | nola_application::WorkStepTransactionPhase::EffectObserved => Err(error_with_display(
            format!(
                "Work step transaction {} requires recovery from phase {:?}",
                prepared.transaction.id, prepared.transaction.phase
            ),
            "The prior side effect is incomplete and was not repeated".to_string(),
        )),
        nola_application::WorkStepTransactionPhase::Prepared => {
            let workspace_checkpoint_id =
                automatic_workspace_checkpoint(request, ctx, effect, transaction_boundary)?;
            let checkpointed = crate::work_step_port::advance_work_step_transaction(
                    &prepared.transaction.id,
                    prepared.transaction.revision,
                    nola_application::WorkStepTransactionPhase::Checkpointed,
                    workspace_checkpoint_id,
                    None,
                    None,
                    None,
                )
                .map_err(|error| {
                    error_with_display(
                        format!("Claim work step execution boundary failed: {error}"),
                        "The side effect was not executed because another worker owns its recovery boundary"
                            .to_string(),
                    )
                })?;
            Ok(Some(checkpointed))
        }
    }
}

pub(crate) fn finish_tool_work_step_transaction(
    transaction: Option<nola_application::WorkStepTransaction>,
    outcome: ToolOutcome,
) -> ToolOutcome {
    let Some(transaction) = transaction else {
        return outcome;
    };
    let result = tool_outcome_transaction_value(&outcome);
    let result_ref = full_output_ref_from_outcome(&outcome);
    let observed = match crate::work_step_port::advance_work_step_transaction(
        &transaction.id,
        transaction.revision,
        nola_application::WorkStepTransactionPhase::EffectObserved,
        transaction.workspace_checkpoint_id.clone(),
        Some(result.clone()),
        result_ref.clone(),
        None,
    ) {
        Ok(value) => value,
        Err(error) => {
            return error_with_display(
                format!("Record observed side effect failed: {error}"),
                "The tool returned, but its side effect requires recovery".to_string(),
            )
        }
    };
    let terminal_phase = if outcome.error {
        nola_application::WorkStepTransactionPhase::Failed
    } else {
        nola_application::WorkStepTransactionPhase::Committed
    };
    let terminal_error = outcome.error.then(|| outcome.text.clone());
    match crate::work_step_port::advance_work_step_transaction(
        &observed.id,
        observed.revision,
        terminal_phase,
        observed.workspace_checkpoint_id,
        Some(result),
        result_ref,
        terminal_error,
    ) {
        Ok(_) => outcome,
        Err(error) => error_with_display(
            format!("Commit work step transaction result failed: {error}"),
            "The tool returned, but its result requires recovery".to_string(),
        ),
    }
}

/// These adapters only edit their declared files; no process/plugin execution.
pub(crate) fn tool_has_bounded_file_writes(name:&str)->bool {
    matches!(name,"write_file"|"edit_file"|"edit_file_lines"|"delete_file")
}

#[cfg(test)]
mod native_sensitive_path_tests {
    #[test]
    fn sensitive_resources_use_both_native_path_separators() {
        for path in [r"C:\Users\fixture\.env", r"\\?\C:\fixture\.ssh\id_rsa", "/fixture/.env"] {
            assert!(super::bash_token_is_sensitive_path(path), "{path}");
        }
        assert!(!super::bash_token_is_sensitive_path(r"C:\fixture\.env.example"));
    }
}
