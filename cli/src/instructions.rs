use sha2::Digest;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config_paths::{expand_home, project_ancestor_directories};
use nola_core::agent::{
    select_active_instructions, AgentInstructionManifest, AgentInstructionSource,
};

const MAX_INSTRUCTION_BYTES: usize = 40_000;
const MAX_TOTAL_INSTRUCTION_CHARS: usize = 100_000;
const MAX_INCLUDE_DEPTH: usize = 5;

#[derive(Debug, Clone)]
pub struct InstructionBundle {
    pub entries: Vec<InstructionEntry>,
    pub manifest: AgentInstructionManifest,
}

#[derive(Debug, Clone)]
pub struct InstructionEntry {
    pub path: PathBuf,
    pub content: String,
    pub source: AgentInstructionSource,
}

#[derive(Debug, Clone)]
struct DiscoveredInstruction {
    path: PathBuf,
    scope_uri: String,
}

impl InstructionBundle {
    pub fn load(project_root: &Path, instructions: &[String]) -> Self {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for discovered in discover_instruction_paths(project_root, instructions) {
            load_instruction_file(
                &discovered.path,
                &discovered.scope_uri,
                &mut seen,
                &mut entries,
                0,
            );
        }
        entries.sort_by(|left, right| {
            left.source
                .kind
                .cmp(&right.source.kind)
                .then(left.source.scope_depth.cmp(&right.source.scope_depth))
                .then(left.source.scope_uri.cmp(&right.source.scope_uri))
                .then(left.source.source_id.cmp(&right.source.source_id))
        });
        enforce_instruction_budget(&mut entries);
        let native_paths = entries.iter().map(|entry| (entry.source.source_id.clone(), entry.path.clone())).collect::<std::collections::HashMap<_, _>>();
        let sources = entries
            .into_iter()
            .map(|entry| entry.source)
            .collect::<Vec<_>>();
        let active_workspace = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        let selection = select_active_instructions(&active_workspace.to_string_lossy(), sources)
            .unwrap_or_else(|_| nola_core::agent::AgentInstructionSelection {
                sources: Vec::new(),
                manifest: AgentInstructionManifest::default(),
            });
        let entries = selection
            .sources
            .iter()
            .map(|source| InstructionEntry {
                path: native_paths[&source.source_id].clone(),
                content: source.content.clone(),
                source: source.clone(),
            })
            .collect();
        Self {
            entries,
            manifest: selection.manifest,
        }
    }

    pub fn to_system_messages(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| {
                let label = instruction_entry_label(&entry.path);
                format!(
                    "{label}: {}\n{}",
                    entry.path.display(),
                    entry.content.trim()
                )
            })
            .collect()
    }
}

fn instruction_entry_label(path: &Path) -> &'static str {
    match path.file_name().and_then(|value| value.to_str()) {
        Some("NOLA.md") | Some("NOLA.local.md") => "Project long-term memory and instructions from",
        _ => "Instructions from",
    }
}

fn discover_instruction_paths(
    project_root: &Path,
    instructions: &[String],
) -> Vec<DiscoveredInstruction> {
    let mut out = Vec::new();
    if let Some(home) = crate::platform_paths::home_dir() {
        let global = home.join(".nola");
        if let Some(path) = preferred_instruction_path(&global, &["NOLA.local.md", "NOLA.md"]) {
            out.push(DiscoveredInstruction {
                path,
                scope_uri: "global://".to_string(),
            });
        }
        if let Some(path) =
            preferred_instruction_path(&global, &["AGENTS.override.md", "AGENTS.md"])
        {
            out.push(DiscoveredInstruction {
                path,
                scope_uri: "global://".to_string(),
            });
        }
    }
    let directories = project_ancestor_directories(project_root);
    for directory in directories {
        let scope_uri = directory.to_string_lossy().into_owned();
        if let Some(path) = preferred_instruction_path(&directory, &["NOLA.local.md", "NOLA.md"]) {
            out.push(DiscoveredInstruction {
                path,
                scope_uri: scope_uri.clone(),
            });
        }
        if let Some(path) =
            preferred_instruction_path(&directory, &["AGENTS.override.md", "AGENTS.md"])
        {
            out.push(DiscoveredInstruction {
                path,
                scope_uri: scope_uri.clone(),
            });
        }
        out.extend(
            markdown_files(directory.join(".nola/rules"))
                .into_iter()
                .map(|path| DiscoveredInstruction {
                    path,
                    scope_uri: scope_uri.clone(),
                }),
        );
    }
    for raw in instructions {
        out.extend(
            resolve_instruction_pattern(project_root, raw)
                .into_iter()
                .map(|path| DiscoveredInstruction {
                    path,
                    scope_uri: project_root.to_string_lossy().into_owned(),
                }),
        );
    }
    out
}

/// Candidate order is authoritative: a local/override file shadows the base
/// file only in the same directory. Ancestor and descendant scopes remain
/// independent so root-to-leaf instruction precedence is still visible to
/// Core rather than being flattened by the Host.
fn preferred_instruction_path(directory: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
}

fn enforce_instruction_budget(entries: &mut Vec<InstructionEntry>) {
    let mut remaining = MAX_TOTAL_INSTRUCTION_CHARS;
    let mut retained = Vec::with_capacity(entries.len());
    for mut entry in entries.drain(..).rev() {
        if remaining < 128 {
            continue;
        }
        let content_chars = entry.content.chars().count();
        if content_chars > remaining {
            entry.content = truncate_chars(&entry.content, remaining);
        }
        remaining = remaining.saturating_sub(entry.content.chars().count());
        let observed_at = entry.source.observed_at.clone();
        entry.source = AgentInstructionSource::scoped_user_instruction(
            entry.source.source_id.clone(),
            entry.source.scope_uri.clone(),
            entry.source.origin_uri.clone(),
            entry.content.clone(),
            entry.source.cacheable,
        );
        entry.source.observed_at = observed_at;
        retained.push(entry);
    }
    retained.reverse();
    *entries = retained;
}

fn markdown_files(dir: PathBuf) -> Vec<PathBuf> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = read_dir
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn resolve_instruction_pattern(project_root: &Path, raw: &str) -> Vec<PathBuf> {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Vec::new();
    }
    let expanded = expand_home(Path::new(raw));
    let path = if expanded.is_absolute() {
        expanded
    } else {
        project_root.join(expanded)
    };
    if raw.contains('*') {
        let Some(parent) = path.parent() else {
            return Vec::new();
        };
        let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
            return Vec::new();
        };
        let mut out = fs::read_dir(parent)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|item| item.to_str())
                    .map(|item| wildcard_match(item, name))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        out.sort();
        return out;
    }
    vec![path]
}

fn load_instruction_file(
    path: &Path,
    scope_uri: &str,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<InstructionEntry>,
    depth: usize,
) {
    if depth > MAX_INCLUDE_DEPTH {
        return;
    }
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical.clone()) {
        return;
    }
    let Ok(raw) = fs::read_to_string(&canonical) else {
        return;
    };
    let (content, include_paths) = parse_instruction_content(&raw, &canonical);
    for include in include_paths {
        load_instruction_file(&include, scope_uri, seen, entries, depth + 1);
    }
    let trimmed = truncate_chars(content.trim(), MAX_INSTRUCTION_BYTES);
    if !trimmed.trim().is_empty() {
        let origin_uri = canonical.to_string_lossy().into_owned();
        let source_id = format!(
            "repository-instruction:{}",
            hex::encode(sha2::Sha256::digest(origin_uri.as_bytes()))
        );
        let mut source = AgentInstructionSource::scoped_user_instruction(
            source_id,
            scope_uri,
            origin_uri,
            trimmed,
            scope_uri == "global://",
        );
        source.observed_at = instruction_observed_at(&canonical);
        entries.push(InstructionEntry {
            path: canonical,
            content: source.content.clone(),
            source,
        });
    }
}

fn instruction_observed_at(path: &Path) -> Option<String> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| format!("unix-ms:{}", duration.as_millis()))
}

fn parse_instruction_content(raw: &str, path: &Path) -> (String, Vec<PathBuf>) {
    let content = strip_frontmatter(raw);
    let content = strip_html_comments(&content);
    let include_paths = extract_include_paths(&content, path);
    (content, include_paths)
}

fn strip_frontmatter(raw: &str) -> String {
    let normalized = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if !normalized.starts_with("---\n") {
        return normalized.to_string();
    }
    let rest = &normalized[4..];
    if let Some(end) = rest.find("\n---\n") {
        return rest[end + 5..].to_string();
    }
    normalized.to_string()
}

fn strip_html_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        if let Some(end) = after.find("-->") {
            rest = &after[end + 3..];
        } else {
            out.push_str(&rest[start..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn extract_include_paths(content: &str, base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for token in line.split_whitespace() {
            let Some(raw) = token.strip_prefix('@') else {
                continue;
            };
            let raw = raw.trim_matches(|ch: char| matches!(ch, ',' | ')' | ']' | '}' | '"' | '\''));
            if raw.is_empty() || raw.starts_with("http://") || raw.starts_with("https://") {
                continue;
            }
            let raw = raw.split('#').next().unwrap_or(raw);
            let expanded = expand_home(Path::new(raw));
            let path = if expanded.is_absolute() {
                expanded
            } else {
                base.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(expanded)
            };
            if is_text_like(&path) {
                out.push(path);
            }
        }
    }
    out
}

fn is_text_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).unwrap_or(""),
        "md" | "txt"
            | "json"
            | "jsonc"
            | "toml"
            | "yaml"
            | "yml"
            | "rs"
            | "py"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "kt"
            | "swift"
            | "go"
            | "java"
            | "sh"
    )
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(80))
        .collect::<String>();
    out.push_str("\n\n[Instruction file truncated by Nola CLI]\n");
    out
}

fn wildcard_match(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return value.starts_with(prefix) && value.ends_with(suffix);
    }
    value == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_frontmatter_and_comments() {
        let raw = "---\npaths: [src/**]\n---\nA\n<!-- hidden -->\nB";
        let (content, _) = parse_instruction_content(raw, Path::new("/tmp/NOLA.md"));
        assert!(content.contains("A"));
        assert!(content.contains("B"));
        assert!(!content.contains("paths:"));
        assert!(!content.contains("hidden"));
    }

    #[test]
    fn discovers_instruction_directories_from_repository_root_to_cwd() {
        let root =
            std::env::temp_dir().join(format!("nola-instruction-tree-{}", std::process::id()));
        let nested = root.join("cli").join("scripts");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();

        let canonical_root = root.canonicalize().unwrap();
        let directories = project_ancestor_directories(&nested);

        assert_eq!(
            directories,
            vec![
                canonical_root.clone(),
                canonical_root.join("cli"),
                canonical_root.join("cli/scripts")
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_preserves_typed_scope_and_stable_root_to_leaf_order() {
        let root =
            std::env::temp_dir().join(format!("nola-typed-instructions-{}", std::process::id()));
        let nested = root.join("app").join("src");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("AGENTS.md"), "root guidance").unwrap();
        fs::write(root.join("app/AGENTS.md"), "app guidance").unwrap();

        let bundle = InstructionBundle::load(&nested, &[]);
        let canonical_root = root.canonicalize().unwrap();
        let repository_sources = bundle
            .entries
            .iter()
            .filter(|entry| entry.path.starts_with(&canonical_root))
            .collect::<Vec<_>>();
        assert_eq!(repository_sources.len(), 2);
        assert_eq!(repository_sources[0].content, "root guidance");
        assert_eq!(repository_sources[1].content, "app guidance");
        assert!(repository_sources.iter().all(|entry| {
            entry.source.kind == nola_core::agent::AgentInstructionKind::UserInstruction
                && !entry.source.content_digest.is_empty()
                && entry.source.observed_at.is_some()
        }));
        assert_eq!(bundle.manifest.sources.len(), bundle.entries.len());
        assert!(bundle
            .manifest
            .sources
            .iter()
            .all(|source| source.selection_reason == "scope_contains_active"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_nola_and_agents_instruction_files_are_loaded() {
        let root = std::env::temp_dir().join(format!(
            "nola-current-instruction-files-{}",
            std::process::id()
        ));
        let nested = root.join("packages/app");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("NOLA.md"), "native guidance").unwrap();
        fs::write(root.join("AGENTS.md"), "agents guidance").unwrap();
        fs::write(nested.join("CLAUDE.md"), "duplicate compatibility guidance").unwrap();

        let bundle = InstructionBundle::load(&nested, &[]);
        let contents = bundle
            .entries
            .iter()
            .map(|entry| entry.content.as_str())
            .collect::<Vec<_>>();

        assert!(contents.contains(&"native guidance"));
        assert!(contents.contains(&"agents guidance"));
        assert!(!contents.contains(&"duplicate compatibility guidance"));
        fs::remove_dir_all(root).unwrap();
    }
}
