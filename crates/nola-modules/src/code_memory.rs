// SPDX-License-Identifier: Apache-2.0
//! Native Rust Tree-sitter AST Code-Memory Engine backed by SQLite
//! (`code_files`, `code_symbols`, `code_symbols_fts`, `code_edges`).
//!
//! Uses real Tree-sitter grammars (`tree-sitter-rust`, `tree-sitter-typescript`,
//! `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-swift`,
//! `tree-sitter-kotlin-ng`, plus `arborium` grammars for Go/Java/C/C++/C#/Ruby/
//! PHP/Dart/Bash/Lua/SQL/HTML/CSS/Svelte/Vue/etc.) on desktop/CLI targets, with a
//! lightweight regex fallback on mobile targets or unknown extensions.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ignore::WalkBuilder;
use nola_store::{CodeEdgeRow, CodeFileRow, CodeSymbolRow, Store};
use nola_tools::{path_within_workspace, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    now_ms, sha256_hex, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_CODE_MEMORY_INDEX: &str = "code_memory_index";

const MAX_INDEXED_FILES: usize = 5_000;
const MAX_INDEXED_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB per source file
const MAX_SNIPPET_CONTEXT_LINES: usize = 120;

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "svelte", "vue", "py", "kt", "kts", "swift",
    "go", "java", "c", "h", "cc", "cpp", "cxx", "hpp", "cs", "rb", "php", "dart", "sh", "bash",
    "lua", "sql", "html", "css", "scss", "json", "toml", "yaml", "yml", "md", "proto", "graphql",
    "hcl", "tf", "xml", "m", "mm",
];

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "build",
    "dist",
    "DerivedData",
    ".build",
    ".swiftpm",
    ".gradle",
    ".next",
    ".svelte-kit",
    ".cache",
    ".venv",
    "venv",
    "Pods",
    "vendor",
    "__pycache__",
];

const MAX_WALK_DEPTH: usize = 32;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_AST_NODES_PER_FILE: usize = 250_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFileEntry {
    pub path: String,
    pub mtime_ms: i64,
    pub size: u64,
    pub symbols: Vec<CodeSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCodeIndex {
    pub workspace_root: String,
    pub indexed_at_ms: i64,
    pub files: BTreeMap<String, IndexedFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStats {
    pub workspace_root: String,
    pub files_indexed: usize,
    pub files_reused: usize,
    pub total_files: usize,
    pub total_symbols: usize,
    pub duration_ms: u64,
}

fn canonical_workspace_key(workspace_root: &Path) -> String {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn detect_language_from_path(path: &Path) -> Option<&'static str> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_ascii_lowercase();
        if lower == "dockerfile" || lower.starts_with("dockerfile.") {
            return Some("dockerfile");
        }
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "swift" => Some("swift"),
        "kt" | "kts" => Some("kotlin"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("cpp"),
        "cs" => Some("c-sharp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "dart" => Some("dart"),
        "sh" | "bash" => Some("bash"),
        "lua" => Some("lua"),
        "sql" => Some("sql"),
        "html" => Some("html"),
        "css" => Some("css"),
        "scss" => Some("scss"),
        "svelte" => Some("svelte"),
        "vue" => Some("vue"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "md" => Some("markdown"),
        "proto" => Some("proto"),
        "graphql" => Some("graphql"),
        "hcl" | "tf" => Some("hcl"),
        "xml" => Some("xml"),
        "m" | "mm" => Some("objc"),
        _ => None,
    }
}

fn is_supported_source_file(path: &Path) -> bool {
    if detect_language_from_path(path).is_some() {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            SUPPORTED_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

fn has_skipped_component(rel_path: &Path) -> bool {
    rel_path.components().any(|c| {
        matches!(c, Component::Normal(name) if name
            .to_str()
            .is_some_and(|s| SKIP_DIR_NAMES.contains(&s)))
    })
}

fn normalize_rel_path(workspace_root: &Path, abs_path: &Path) -> String {
    let rel = abs_path.strip_prefix(workspace_root).unwrap_or(abs_path);
    rel.to_string_lossy().replace('\\', "/")
}

fn walk_source_files(workspace_root: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .hidden(false)
        .follow_links(false)
        .max_depth(Some(MAX_WALK_DEPTH))
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .ignore(true)
        .parents(false)
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.path_is_symlink() {
                return false;
            }
            if let Some(name) = entry.file_name().to_str() {
                if SKIP_DIR_NAMES.contains(&name) {
                    return false;
                }
            }
            true
        });

    for entry in builder.build().flatten() {
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
        if entry.path_is_symlink() {
            continue;
        }
        let path = entry.path();
        let Ok(sym_meta) = fs::symlink_metadata(path) else {
            continue;
        };
        if !sym_meta.is_file() || !is_supported_source_file(path) {
            continue;
        }
        let rel = path.strip_prefix(workspace_root).unwrap_or(path);
        if has_skipped_component(rel) {
            continue;
        }
        files.push((normalize_rel_path(workspace_root, path), path.to_path_buf()));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

struct ParsedFileData {
    language: String,
    symbols: Vec<CodeSymbolRow>,
    edges: Vec<CodeEdgeRow>,
}

// --- Tree-sitter AST Extractor (Desktop / CLI) ---
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod ts_ast {
    use super::*;
    use arborium::tree_sitter::{Language, Node, Parser};
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static PARSERS: RefCell<HashMap<&'static str, Parser>> = RefCell::new(HashMap::new());
    }

    fn grammar_for(language: &str) -> Option<Language> {
        match language {
            "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
            "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
            "python" => Some(tree_sitter_python::LANGUAGE.into()),
            "swift" => Some(tree_sitter_swift::LANGUAGE.into()),
            "kotlin" => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            other => arborium::get_language(other),
        }
    }

    pub(super) fn extract_ast(
        ws_key: &str,
        rel_path: &str,
        language: &'static str,
        source: &[u8],
    ) -> Option<ParsedFileData> {
        let tree = PARSERS.with(|cache| {
            let mut parsers = cache.borrow_mut();
            if !parsers.contains_key(language) {
                let lang = grammar_for(language)?;
                let mut parser = Parser::new();
                parser.set_language(&lang).ok()?;
                parsers.insert(language, parser);
            }
            parsers.get_mut(language)?.parse(source, None)
        })?;

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut seen_edges: HashSet<(String, String, i64, &'static str)> = HashSet::new();
        let mut scope_stack: Vec<(String, bool)> = Vec::new(); // (name, is_type_container)

        visit_node(
            tree.root_node(),
            ws_key,
            rel_path,
            language,
            source,
            &mut scope_stack,
            &mut symbols,
            &mut edges,
            &mut seen_edges,
        );

        Some(ParsedFileData {
            language: language.to_string(),
            symbols,
            edges,
        })
    }

    enum VisitAction<'a> {
        Enter(Node<'a>),
        ExitScope,
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_node(
        root: Node<'_>,
        ws_key: &str,
        rel_path: &str,
        language: &str,
        source: &[u8],
        scope_stack: &mut Vec<(String, bool)>,
        symbols: &mut Vec<CodeSymbolRow>,
        edges: &mut Vec<CodeEdgeRow>,
        seen_edges: &mut HashSet<(String, String, i64, &'static str)>,
    ) {
        let mut stack: Vec<VisitAction<'_>> = Vec::with_capacity(256);
        let mut children_buf: Vec<Node<'_>> = Vec::new();
        stack.push(VisitAction::Enter(root));
        let mut visited_nodes: usize = 0;

        while let Some(action) = stack.pop() {
            match action {
                VisitAction::ExitScope => {
                    scope_stack.pop();
                }
                VisitAction::Enter(node) => {
                    visited_nodes += 1;
                    if visited_nodes > MAX_AST_NODES_PER_FILE {
                        break;
                    }

                    collect_references(
                        node,
                        ws_key,
                        rel_path,
                        source,
                        scope_stack,
                        edges,
                        seen_edges,
                    );

                    let mut pushed_scope = false;
                    let in_type = scope_stack.iter().any(|(_, is_t)| *is_t);

                    // Rust `impl Foo` / `impl Trait for Foo` pushes `Foo` as type scope
                    if node.kind() == "impl_item" {
                        if let Some(type_node) = node.child_by_field_name("type") {
                            if let Some(type_name) = declared_type_name(type_node, source) {
                                scope_stack.push((type_name, true));
                                pushed_scope = true;
                            }
                        }
                    }

                    // JS/TS named arrow functions: `const foo = (...) => ...`
                    if node.kind() == "variable_declarator" {
                        if let Some(val) = node.child_by_field_name("value") {
                            if val.kind() == "arrow_function" || val.kind() == "function_expression"
                            {
                                if let Some(name_node) = node.child_by_field_name("name") {
                                    if let Ok(name) = name_node.utf8_text(source) {
                                        let name = name.trim();
                                        if !name.is_empty() {
                                            let kind = if in_type { "method" } else { "function" };
                                            let qualified =
                                                build_qualified_name(rel_path, scope_stack, name);
                                            let start_line = (node.start_position().row + 1) as i64;
                                            let end_line = (node.end_position().row + 1) as i64;
                                            let sig = first_line(node, source);
                                            let id = sha256_hex(
                                                format!(
                                                    "{ws_key}:{rel_path}:{qualified}:{start_line}"
                                                )
                                                .as_bytes(),
                                            );
                                            symbols.push(CodeSymbolRow {
                                                id,
                                                workspace_root: ws_key.to_string(),
                                                rel_path: rel_path.to_string(),
                                                name: name.to_string(),
                                                qualified_name: qualified,
                                                kind: kind.to_string(),
                                                language: language.to_string(),
                                                start_line,
                                                end_line,
                                                signature: sig,
                                            });
                                            scope_stack.push((name.to_string(), false));
                                            pushed_scope = true;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(raw_kind) = node_symbol_kind(node, language) {
                        if let Some(name_node) = declaration_name_node(node) {
                            if let Ok(raw_name) = name_node.utf8_text(source) {
                                let name = raw_name.trim();
                                if !name.is_empty() {
                                    let refined = refine_swift_kind(node, source, raw_kind);
                                    let effective_kind = if refined == "function" && in_type {
                                        "method"
                                    } else {
                                        refined
                                    };
                                    let qualified =
                                        build_qualified_name(rel_path, scope_stack, name);
                                    let start_line = (node.start_position().row + 1) as i64;
                                    let end_line = (node.end_position().row + 1) as i64;
                                    let sig = callable_or_first_line(node, source, effective_kind);
                                    let id = sha256_hex(
                                        format!(
                                            "{ws_key}:{rel_path}:{qualified}:{start_line}:{}",
                                            symbols.len()
                                        )
                                        .as_bytes(),
                                    );
                                    symbols.push(CodeSymbolRow {
                                        id,
                                        workspace_root: ws_key.to_string(),
                                        rel_path: rel_path.to_string(),
                                        name: name.to_string(),
                                        qualified_name: qualified,
                                        kind: effective_kind.to_string(),
                                        language: language.to_string(),
                                        start_line,
                                        end_line,
                                        signature: sig,
                                    });
                                    if is_container_kind(node.kind()) && !pushed_scope {
                                        scope_stack.push((
                                            name.to_string(),
                                            is_type_container(node.kind()),
                                        ));
                                        pushed_scope = true;
                                    }
                                }
                            }
                        }
                    }

                    if pushed_scope {
                        stack.push(VisitAction::ExitScope);
                    }

                    children_buf.clear();
                    let mut cursor = node.walk();
                    children_buf.extend(node.named_children(&mut cursor));
                    for child in children_buf.drain(..).rev() {
                        stack.push(VisitAction::Enter(child));
                    }
                }
            }
        }
        scope_stack.clear();
    }

    fn build_qualified_name(
        rel_path: &str,
        scope_stack: &[(String, bool)],
        name: &str,
    ) -> String {
        if scope_stack.is_empty() {
            format!("{rel_path}::{name}")
        } else {
            let scope: Vec<&str> = scope_stack.iter().map(|(s, _)| s.as_str()).collect();
            format!("{rel_path}::{}::{name}", scope.join("::"))
        }
    }

    fn node_symbol_kind(node: Node<'_>, language: &str) -> Option<&'static str> {
        match node.kind() {
            "function_item"
            | "function_signature_item"
            | "function_definition"
            | "function_declaration"
            | "function_signature"
            | "local_function"
            | "function" => {
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "method_signature")
                {
                    None
                } else {
                    Some("function")
                }
            }
            "method_definition"
            | "method_declaration"
            | "method"
            | "method_signature"
            | "abstract_method_signature"
            | "protocol_function_declaration" => Some("method"),
            "struct_item" | "struct_specifier" => Some("struct"),
            "enum_item" | "enum_declaration" | "enum_specifier" => Some("enum"),
            "trait_item" => Some("trait"),
            "interface_declaration" | "protocol_declaration" => Some("interface"),
            "type_item" | "type_alias_declaration" | "typealias_declaration" => Some("type"),
            "type_declaration" if language == "go" => Some("type"),
            "class_definition"
            | "class_declaration"
            | "class_specifier"
            | "class"
            | "abstract_class_declaration"
            | "object_declaration" => Some("class"),
            "mod_item" | "module" | "namespace_definition" => Some("module"),
            "const_item" | "static_item" => Some("constant"),
            "macro_definition" => Some("macro"),
            _ => None,
        }
    }

    fn refine_swift_kind<'a>(node: Node<'_>, source: &[u8], fallback: &'a str) -> &'a str {
        let Some(decl_kind) = node.child_by_field_name("declaration_kind") else {
            return fallback;
        };
        match decl_kind.utf8_text(source).ok().map(str::trim) {
            Some("struct") => "struct",
            Some("enum") => "enum",
            Some("protocol") => "interface",
            _ => fallback,
        }
    }

    fn declaration_name_node(node: Node<'_>) -> Option<Node<'_>> {
        if let Some(name) = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("function"))
        {
            return find_identifier(name).or(Some(name));
        }
        if let Some(declarator) = node.child_by_field_name("declarator") {
            if let Some(name) = find_identifier(declarator) {
                return Some(name);
            }
        }
        let mut cursor = node.walk();
        let found = node.named_children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "identifier"
                    | "field_identifier"
                    | "type_identifier"
                    | "simple_identifier"
                    | "property_identifier"
                    | "name"
                    | "constant"
                    | "type_spec"
            )
        });
        found.and_then(|n| find_identifier(n).or(Some(n)))
    }

    fn find_identifier(root: Node<'_>) -> Option<Node<'_>> {
        let mut stack = vec![root];
        let mut children_buf = Vec::new();
        let mut steps = 0usize;
        while let Some(node) = stack.pop() {
            steps += 1;
            if steps > 10_000 {
                break;
            }
            if matches!(
                node.kind(),
                "identifier"
                    | "field_identifier"
                    | "type_identifier"
                    | "simple_identifier"
                    | "property_identifier"
                    | "name"
                    | "constant"
            ) {
                return Some(node);
            }
            children_buf.clear();
            let mut cursor = node.walk();
            children_buf.extend(node.named_children(&mut cursor));
            for child in children_buf.drain(..).rev() {
                stack.push(child);
            }
        }
        None
    }

    fn declared_type_name(root: Node<'_>, source: &[u8]) -> Option<String> {
        let mut stack = vec![root];
        let mut children_buf = Vec::new();
        let mut steps = 0usize;
        while let Some(node) = stack.pop() {
            steps += 1;
            if steps > 10_000 {
                break;
            }
            if matches!(
                node.kind(),
                "type_identifier" | "identifier" | "simple_identifier"
            ) {
                if let Some(name) = node
                    .utf8_text(source)
                    .ok()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                {
                    return Some(name);
                }
            }
            children_buf.clear();
            let mut cursor = node.walk();
            children_buf.extend(node.named_children(&mut cursor));
            for child in children_buf.drain(..).rev() {
                stack.push(child);
            }
        }
        None
    }

    fn is_container_kind(kind: &str) -> bool {
        matches!(
            kind,
            "struct_item"
                | "struct_specifier"
                | "enum_item"
                | "enum_declaration"
                | "trait_item"
                | "class_definition"
                | "class_declaration"
                | "class_specifier"
                | "class"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "protocol_declaration"
                | "object_declaration"
                | "mod_item"
                | "module"
                | "namespace_definition"
                | "function_item"
                | "function_definition"
                | "function_declaration"
                | "method_definition"
                | "method_declaration"
                | "method"
        )
    }

    fn is_type_container(kind: &str) -> bool {
        matches!(
            kind,
            "struct_item"
                | "struct_specifier"
                | "enum_item"
                | "enum_declaration"
                | "trait_item"
                | "class_definition"
                | "class_declaration"
                | "class_specifier"
                | "class"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "protocol_declaration"
                | "object_declaration"
        )
    }

    fn callable_or_first_line(node: Node<'_>, source: &[u8], kind: &str) -> String {
        if matches!(kind, "function" | "method") {
            let end = node
                .child_by_field_name("body")
                .map(|b| b.start_byte())
                .unwrap_or_else(|| node.end_byte());
            let sig: String = String::from_utf8_lossy(&source[node.start_byte()..end])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(200)
                .collect();
            if !sig.is_empty() {
                return sig;
            }
        }
        first_line(node, source)
    }

    fn first_line(node: Node<'_>, source: &[u8]) -> String {
        let slice = &source[node.start_byte()..node.end_byte()];
        String::from_utf8_lossy(slice)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(200)
            .collect()
    }

    fn terminal_identifier(value: &str) -> Option<String> {
        value
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|p| !p.is_empty())
            .next_back()
            .filter(|s| s.len() > 1 && !s.chars().all(|c| c.is_ascii_digit()))
            .map(ToOwned::to_owned)
    }

    fn collect_references(
        node: Node<'_>,
        ws_key: &str,
        rel_path: &str,
        source: &[u8],
        scope_stack: &[(String, bool)],
        edges: &mut Vec<CodeEdgeRow>,
        seen_edges: &mut HashSet<(String, String, i64, &'static str)>,
    ) {
        let from_symbol = scope_stack
            .last()
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| rel_path.to_string());

        if matches!(
            node.kind(),
            "call"
                | "call_expression"
                | "function_call"
                | "method_invocation"
                | "invocation_expression"
        ) {
            let target = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("callee"))
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.child_by_field_name("target"))
                .or_else(|| {
                    let mut c = node.walk();
                    let found = node.named_children(&mut c).next();
                    found
                });
            if let Some(target_node) = target {
                if let Ok(text) = target_node.utf8_text(source) {
                    if let Some(to_symbol) = terminal_identifier(text) {
                        if to_symbol != from_symbol {
                            let line = (node.start_position().row + 1) as i64;
                            if seen_edges.insert((
                                from_symbol.clone(),
                                to_symbol.clone(),
                                line,
                                "calls",
                            )) {
                                edges.push(CodeEdgeRow {
                                    workspace_root: ws_key.to_string(),
                                    from_symbol,
                                    to_symbol,
                                    rel_path: rel_path.to_string(),
                                    line,
                                    edge_kind: "calls".to_string(),
                                });
                            }
                        }
                    }
                }
            }
            return;
        }

        if matches!(
            node.kind(),
            "use_declaration"
                | "import_statement"
                | "import_from_statement"
                | "import_header"
                | "import_declaration"
                | "using_directive"
        ) {
            let mut stack = vec![node];
            while let Some(cur) = stack.pop() {
                let mut cursor = cur.walk();
                for child in cur.named_children(&mut cursor) {
                    if matches!(
                        child.kind(),
                        "identifier" | "type_identifier" | "simple_identifier" | "name"
                    ) {
                        if let Ok(text) = child.utf8_text(source) {
                            let to_symbol = text.trim().to_string();
                            let line = (child.start_position().row + 1) as i64;
                            if to_symbol.len() > 1
                                && seen_edges.insert((
                                    rel_path.to_string(),
                                    to_symbol.clone(),
                                    line,
                                    "imports",
                                ))
                            {
                                edges.push(CodeEdgeRow {
                                    workspace_root: ws_key.to_string(),
                                    from_symbol: rel_path.to_string(),
                                    to_symbol,
                                    rel_path: rel_path.to_string(),
                                    line,
                                    edge_kind: "imports".to_string(),
                                });
                            }
                        }
                    } else {
                        stack.push(child);
                    }
                }
            }
        }
    }
}

// --- Regex Fallback Extractor (Mobile or unknown grammars) ---
struct SymbolMatchers {
    rust: Vec<(&'static str, Regex)>,
    ts_js: Vec<(&'static str, Regex)>,
    python: Vec<(&'static str, Regex)>,
    go: Vec<(&'static str, Regex)>,
    java_kt: Vec<(&'static str, Regex)>,
    swift: Vec<(&'static str, Regex)>,
    c_cpp: Vec<(&'static str, Regex)>,
}

fn matchers() -> &'static SymbolMatchers {
    static MATCHERS: OnceLock<SymbolMatchers> = OnceLock::new();
    MATCHERS.get_or_init(|| SymbolMatchers {
        rust: vec![
            (
                "function",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?(?:extern\s+\S+\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "trait",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "type",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "module",
                Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        ts_js: vec![
            (
                "function",
                Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "type",
                Regex::new(r"^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*=").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:export\s+)?(?:const\s+)?enum\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "export",
                Regex::new(r"^\s*export\s+(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
        ],
        python: vec![
            (
                "function",
                Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        go: vec![
            (
                "method",
                Regex::new(r"^\s*func\s*\([^)]+\)\s*([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^\s*func\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+struct\b").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+interface\b").unwrap(),
            ),
        ],
        java_kt: vec![
            (
                "class",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|abstract|final|open|data|sealed|static)\s+)*(?:class|object)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|sealed)\s+)*interface\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal)\s+)*enum\s+(?:class\s+)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|override|suspend|inline|open|final|static)\s+)*fun\s+(?:<[^>]+>\s+)?(?:[A-Za-z0-9_<>?]+\.)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "method",
                Regex::new(r"^\s*(?:(?:public|private|protected|static|final|synchronized|abstract)\s+)+[A-Za-z0-9_<>\[\]?]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap(),
            ),
        ],
        swift: vec![
            (
                "function",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|static|class|mutating|nonmutating|override|final)\s+)*func\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|final)\s+)*struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|final)\s+)*(?:class|actor)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|indirect)\s+)*enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open)\s+)*protocol\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        c_cpp: vec![
            (
                "struct",
                Regex::new(r"^\s*(?:typedef\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:typedef\s+)?enum\s+(?:class\s+)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^\s*(?:static\s+|inline\s+|extern\s+|virtual\s+|constexpr\s+)*[A-Za-z_][A-Za-z0-9_:<>*&\s]{0,60}\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*(?:const\s*)?(?:\{|$)").unwrap(),
            ),
        ],
    })
}

fn extract_regex_fallback(
    ws_key: &str,
    rel_path: &str,
    ext: &str,
    language: &str,
    content: &str,
) -> ParsedFileData {
    let m = matchers();
    let rules: &[(&str, Regex)] = match ext {
        "rs" => &m.rust,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "svelte" | "vue" => &m.ts_js,
        "py" => &m.python,
        "go" => &m.go,
        "java" | "kt" | "kts" => &m.java_kt,
        "swift" => &m.swift,
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" => &m.c_cpp,
        _ => &[],
    };

    let mut symbols = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let is_rust_attr = ext == "rs" && (trimmed.starts_with("#[") || trimmed.starts_with("#!["));
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || (trimmed.starts_with('#') && !is_rust_attr)
        {
            continue;
        }
        for (kind, re) in rules {
            if let Some(caps) = re.captures(line) {
                if let Some(name_match) = caps.get(1) {
                    let name = name_match.as_str();
                    if matches!(name, "if" | "for" | "while" | "switch" | "catch" | "return") {
                        break;
                    }
                    let start_line = (idx + 1) as i64;
                    let qualified_name = format!("{rel_path}::{name}");
                    let id = sha256_hex(
                        format!("{ws_key}:{rel_path}:{qualified_name}:{start_line}").as_bytes(),
                    );
                    let signature: String = trimmed.chars().take(200).collect();
                    symbols.push(CodeSymbolRow {
                        id,
                        workspace_root: ws_key.to_string(),
                        rel_path: rel_path.to_string(),
                        name: name.to_string(),
                        qualified_name,
                        kind: (*kind).to_string(),
                        language: language.to_string(),
                        start_line,
                        end_line: start_line,
                        signature,
                    });
                    break;
                }
            }
        }
    }
    ParsedFileData {
        language: language.to_string(),
        symbols,
        edges: Vec::new(),
    }
}

fn parse_source_file(
    ws_key: &str,
    rel_path: &str,
    abs_path: &Path,
    content: &str,
) -> ParsedFileData {
    let ext = abs_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let detected = detect_language_from_path(abs_path);

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Some(lang) = detected {
        if let Some(parsed) = ts_ast::extract_ast(ws_key, rel_path, lang, content.as_bytes()) {
            // For Svelte/Vue templates where embedded <script> blocks need fallback symbols if AST yields none
            if !parsed.symbols.is_empty() || !matches!(lang, "svelte" | "vue") {
                return parsed;
            }
        }
    }

    let lang_label = detected.unwrap_or(if ext.is_empty() { "unknown" } else { ext.as_str() });
    extract_regex_fallback(ws_key, rel_path, &ext, lang_label, content)
}

/// Index or incrementally update workspace source symbols & call/import edges in SQLite
/// (`code_files`, `code_symbols`, `code_symbols_fts`, `code_edges`), skipping unchanged files
/// by SHA-256 content hash and pruning deleted files.
pub fn index_workspace(
    store: &Store,
    workspace_root: &Path,
    force_full: bool,
) -> Result<IndexStats, String> {
    let start = Instant::now();
    if !workspace_root.exists() {
        return Err(format!(
            "workspace root '{}' does not exist",
            workspace_root.display()
        ));
    }

    let ws_key = canonical_workspace_key(workspace_root);
    let existing_hashes = if force_full {
        std::collections::HashMap::new()
    } else {
        store
            .get_code_file_hashes(&ws_key)
            .map_err(|e| e.to_string())?
    };

    let walked = walk_source_files(workspace_root);
    let mut existing_rel_paths = HashSet::with_capacity(walked.len());
    let mut files_indexed = 0usize;
    let mut files_reused = 0usize;
    let ts_now = now_ms();

    for (rel_path, abs_path) in walked {
        let Ok(meta) = fs::metadata(&abs_path) else {
            continue;
        };
        if meta.len() > MAX_INDEXED_FILE_BYTES {
            continue;
        }
        let Ok(raw_bytes) = fs::read(&abs_path) else {
            continue;
        };
        let Ok(content) = String::from_utf8(raw_bytes) else {
            continue;
        };
        let sha = sha256_hex(content.as_bytes());
        existing_rel_paths.insert(rel_path.clone());

        if !force_full && existing_hashes.get(&rel_path).is_some_and(|h| h == &sha) {
            files_reused += 1;
            continue;
        }

        let parsed = parse_source_file(&ws_key, &rel_path, &abs_path, &content);
        let file_row = CodeFileRow {
            workspace_root: ws_key.clone(),
            rel_path: rel_path.clone(),
            sha256: sha,
            language: parsed.language,
            line_count: content.lines().count() as i64,
            updated_at_ms: ts_now,
        };
        store
            .replace_code_file_index(&file_row, &parsed.symbols, &parsed.edges)
            .map_err(|e| e.to_string())?;
        files_indexed += 1;
    }

    store
        .prune_missing_code_files(&ws_key, &existing_rel_paths)
        .map_err(|e| e.to_string())?;

    let stats = store
        .code_index_stats(&ws_key)
        .map_err(|e| e.to_string())?;

    Ok(IndexStats {
        workspace_root: workspace_root.display().to_string(),
        files_indexed,
        files_reused,
        total_files: stats.file_count.max(0) as usize,
        total_symbols: stats.symbol_count.max(0) as usize,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

fn ensure_indexed(store: &Store, workspace_root: &Path) -> Result<String, String> {
    let ws_key = canonical_workspace_key(workspace_root);
    let stats = store
        .code_index_stats(&ws_key)
        .map_err(|e| e.to_string())?;
    if stats.file_count == 0 {
        index_workspace(store, workspace_root, false)?;
    }
    Ok(ws_key)
}

/// Search indexed symbols via SQLite FTS5 (`code_symbols_fts`) with ranking and optional filters.
pub fn search_symbols(
    store: &Store,
    workspace_root: &Path,
    query: &str,
    kind: Option<&str>,
    path_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let ws_key = ensure_indexed(store, workspace_root)?;
    let q_trimmed = query.trim();
    let q_lower = q_trimmed.to_ascii_lowercase();
    let kind_norm = kind
        .map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| !k.is_empty());
    let path_lower = path_filter
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty());
    let cap = limit.clamp(1, 200);
    let fetch_cap = if path_lower.is_some() {
        (cap * 5).min(1_000)
    } else {
        (cap * 2).min(400)
    };

    let rows = store
        .search_code_symbols(&ws_key, q_trimmed, kind_norm.as_deref(), fetch_cap)
        .map_err(|e| e.to_string())?;

    let mut scored: Vec<(u32, CodeSymbolRow)> = Vec::new();
    for sym in rows {
        if let Some(ref pf) = path_lower {
            if !sym.rel_path.to_ascii_lowercase().contains(pf) {
                continue;
            }
        }
        let score = if q_lower.is_empty() {
            10
        } else {
            let name_l = sym.name.to_ascii_lowercase();
            let qname_l = sym.qualified_name.to_ascii_lowercase();
            let sig_l = sym.signature.to_ascii_lowercase();
            if name_l == q_lower {
                100
            } else if name_l.starts_with(&q_lower) {
                80
            } else if name_l.contains(&q_lower) {
                60
            } else if qname_l.contains(&q_lower) {
                45
            } else if sig_l.contains(&q_lower) {
                35
            } else {
                20
            }
        };
        scored.push((score, sym));
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.rel_path.cmp(&b.1.rel_path))
            .then_with(|| a.1.start_line.cmp(&b.1.start_line))
    });

    Ok(scored
        .into_iter()
        .take(cap)
        .map(|(score, sym)| {
            json!({
                "name": sym.name,
                "qualified_name": sym.qualified_name,
                "kind": sym.kind,
                "language": sym.language,
                "file": sym.rel_path,
                "line": sym.start_line,
                "end_line": sym.end_line,
                "signature": sym.signature,
                "score": score
            })
        })
        .collect())
}

/// Bounded fast literal or regex text search across workspace source files.
pub fn search_text(
    workspace_root: &Path,
    query: &str,
    is_regex: bool,
    path_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("query must not be empty".into());
    }
    let compiled = if is_regex {
        Some(Regex::new(trimmed).map_err(|e| format!("invalid regex: {e}"))?)
    } else {
        None
    };
    let path_lower = path_filter.map(|p| p.trim().to_ascii_lowercase());
    let cap = limit.clamp(1, 200);
    let files = walk_source_files(workspace_root);
    let mut hits = Vec::new();
    let needle_lower = trimmed.to_ascii_lowercase();

    for (rel_path, abs_path) in files {
        if hits.len() >= cap {
            break;
        }
        if let Some(ref pf) = path_lower {
            if !pf.is_empty() && !rel_path.to_ascii_lowercase().contains(pf) {
                continue;
            }
        }
        let Ok(meta) = fs::metadata(&abs_path) else {
            continue;
        };
        if meta.len() > MAX_INDEXED_FILE_BYTES {
            continue;
        }
        let Ok(content) = fs::read_to_string(&abs_path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            let matched = match &compiled {
                Some(re) => re.is_match(line),
                None => line.to_ascii_lowercase().contains(&needle_lower),
            };
            if matched {
                let snippet: String = line.chars().take(240).collect();
                hits.push(json!({
                    "file": rel_path,
                    "line": idx + 1,
                    "snippet": snippet
                }));
                if hits.len() >= cap {
                    break;
                }
            }
        }
    }
    Ok(hits)
}

/// Read the exact source snippet around a symbol's file and line range.
pub fn read_symbol_snippet(
    store: &Store,
    workspace_root: &Path,
    name: Option<&str>,
    file: Option<&str>,
    line: Option<usize>,
    context_lines: usize,
) -> Result<Value, String> {
    let mut resolved_file = file
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut resolved_line = line;
    let mut resolved_symbol: Option<Value> = None;

    if resolved_file.is_none() || resolved_line.is_none() {
        let Some(sym_name) = name.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("provide either 'name' or both 'file' and 'line'".into());
        };
        let matches = search_symbols(
            store,
            workspace_root,
            sym_name,
            None,
            resolved_file.as_deref(),
            5,
        )?;
        let Some(best) = matches.into_iter().next() else {
            return Err(format!("symbol '{sym_name}' not found in workspace index"));
        };
        resolved_file = best
            .get("file")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        resolved_line = best
            .get("line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        resolved_symbol = Some(best);
    }

    let rel_file = resolved_file.ok_or_else(|| "missing file path".to_string())?;
    let target_line = resolved_line.unwrap_or(1).max(1);
    let abs_path = workspace_root.join(&rel_file);
    if !path_within_workspace(workspace_root, &abs_path) {
        return Err(format!("path '{rel_file}' escapes workspace root"));
    }
    let content = fs::read_to_string(&abs_path)
        .map_err(|e| format!("failed to read '{rel_file}': {e}"))?;
    let all_lines: Vec<&str> = content.lines().collect();
    if all_lines.is_empty() {
        return Ok(json!({
            "file": rel_file,
            "start_line": 1,
            "end_line": 0,
            "target_line": target_line,
            "symbol": resolved_symbol,
            "snippet": ""
        }));
    }

    let ctx = context_lines.clamp(1, MAX_SNIPPET_CONTEXT_LINES);
    let start_line = target_line.saturating_sub(ctx / 3).max(1);
    let end_line = (target_line + ctx).min(all_lines.len());
    let mut snippet = String::new();
    for line_no in start_line..=end_line {
        let text = all_lines.get(line_no - 1).copied().unwrap_or("");
        snippet.push_str(&format!("{line_no:>5} | {text}\n"));
    }

    Ok(json!({
        "file": rel_file,
        "start_line": start_line,
        "end_line": end_line,
        "target_line": target_line,
        "symbol": resolved_symbol,
        "snippet": snippet
    }))
}

/// Traverse callers and/or callees in the SQLite `code_edges` call graph (`depth: 1..=4`).
pub fn trace_symbol(
    store: &Store,
    workspace_root: &Path,
    symbol: &str,
    direction: &str,
    depth: usize,
    limit: usize,
) -> Result<Value, String> {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return Err("symbol must not be empty".into());
    }
    let dir = match direction.trim().to_ascii_lowercase().as_str() {
        "callers" => "callers",
        "callees" => "callees",
        "both" | "" => "both",
        other => {
            return Err(format!(
                "invalid direction '{other}' (expected 'callers', 'callees', or 'both')"
            ));
        }
    };
    let max_depth = depth.clamp(1, 4);
    let edge_cap = limit.clamp(1, 200);
    let ws_key = ensure_indexed(store, workspace_root)?;

    // Resolve seed symbol name(s) via index if user passed a qualified name
    let mut seed_names = BTreeSet::new();
    seed_names.insert(trimmed.to_string());
    if let Some(short) = trimmed.rsplit("::").next() {
        if !short.is_empty() {
            seed_names.insert(short.to_string());
        }
    }

    let mut callers: Vec<Value> = Vec::new();
    let mut callees: Vec<Value> = Vec::new();
    let mut seen_caller_edges: HashSet<(String, String, String, i64)> = HashSet::new();
    let mut seen_callee_edges: HashSet<(String, String, String, i64)> = HashSet::new();

    if matches!(dir, "callers" | "both") {
        let mut queue: VecDeque<(String, usize)> =
            seed_names.iter().map(|s| (s.clone(), 0)).collect();
        let mut visited: HashSet<String> = seed_names.iter().cloned().collect();
        while let Some((cur, d)) = queue.pop_front() {
            if d >= max_depth || callers.len() >= edge_cap {
                break;
            }
            let incoming = store
                .list_code_edges_to(&ws_key, &cur, edge_cap)
                .map_err(|e| e.to_string())?;
            for edge in incoming {
                if callers.len() >= edge_cap {
                    break;
                }
                let dedup_key = (
                    edge.from_symbol.clone(),
                    edge.to_symbol.clone(),
                    edge.rel_path.clone(),
                    edge.line,
                );
                if seen_caller_edges.insert(dedup_key) {
                    callers.push(json!({
                        "depth": d + 1,
                        "from": edge.from_symbol,
                        "to": edge.to_symbol,
                        "file": edge.rel_path,
                        "line": edge.line,
                        "kind": edge.edge_kind
                    }));
                    if d + 1 < max_depth && visited.insert(edge.from_symbol.clone()) {
                        queue.push_back((edge.from_symbol, d + 1));
                    }
                }
            }
        }
    }

    if matches!(dir, "callees" | "both") {
        let mut queue: VecDeque<(String, usize)> =
            seed_names.iter().map(|s| (s.clone(), 0)).collect();
        let mut visited: HashSet<String> = seed_names.iter().cloned().collect();
        while let Some((cur, d)) = queue.pop_front() {
            if d >= max_depth || callees.len() >= edge_cap {
                break;
            }
            let outgoing = store
                .list_code_edges_from(&ws_key, &cur, edge_cap)
                .map_err(|e| e.to_string())?;
            for edge in outgoing {
                if callees.len() >= edge_cap {
                    break;
                }
                let dedup_key = (
                    edge.from_symbol.clone(),
                    edge.to_symbol.clone(),
                    edge.rel_path.clone(),
                    edge.line,
                );
                if seen_callee_edges.insert(dedup_key) {
                    callees.push(json!({
                        "depth": d + 1,
                        "from": edge.from_symbol,
                        "to": edge.to_symbol,
                        "file": edge.rel_path,
                        "line": edge.line,
                        "kind": edge.edge_kind
                    }));
                    if d + 1 < max_depth && visited.insert(edge.to_symbol.clone()) {
                        queue.push_back((edge.to_symbol, d + 1));
                    }
                }
            }
        }
    }

    Ok(json!({
        "symbol": trimmed,
        "direction": dir,
        "depth": max_depth,
        "caller_count": callers.len(),
        "callee_count": callees.len(),
        "callers": callers,
        "callees": callees
    }))
}

fn module_dir_of(rel_path: &str) -> String {
    match rel_path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        _ => ".".to_string(),
    }
}

/// Produce a directory/module/language symbol & cross-file dependency architecture overview.
pub fn codebase_architecture(
    store: &Store,
    workspace_root: &Path,
    max_modules: usize,
) -> Result<Value, String> {
    let ws_key = ensure_indexed(store, workspace_root)?;
    let stats = store
        .code_index_stats(&ws_key)
        .map_err(|e| e.to_string())?;
    let file_hashes = store
        .get_code_file_hashes(&ws_key)
        .map_err(|e| e.to_string())?;
    let symbols = store
        .search_code_symbols(&ws_key, "", None, 20_000)
        .map_err(|e| e.to_string())?;

    let mut dir_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut dir_symbols: BTreeMap<String, usize> = BTreeMap::new();
    let mut dir_languages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut dir_kinds: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut sym_to_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for rel_path in file_hashes.keys() {
        let dir = module_dir_of(rel_path);
        dir_files.entry(dir).or_default().insert(rel_path.clone());
    }
    for sym in &symbols {
        let dir = module_dir_of(&sym.rel_path);
        dir_files
            .entry(dir.clone())
            .or_default()
            .insert(sym.rel_path.clone());
        *dir_symbols.entry(dir.clone()).or_insert(0) += 1;
        dir_languages
            .entry(dir.clone())
            .or_default()
            .insert(sym.language.clone());
        *dir_kinds
            .entry(dir)
            .or_default()
            .entry(sym.kind.clone())
            .or_insert(0) += 1;
        sym_to_files
            .entry(sym.name.clone())
            .or_default()
            .insert(sym.rel_path.clone());
    }

    // Compute cross-file dependencies from indexed edges (bounded)
    let mut cross_file_deps: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut checked_sources: HashSet<String> = HashSet::new();
    for sym in symbols.iter().take(500) {
        if !checked_sources.insert(sym.name.clone()) {
            continue;
        }
        if let Ok(edges) = store.list_code_edges_from(&ws_key, &sym.name, 50) {
            for edge in edges {
                if let Some(target_files) = sym_to_files.get(&edge.to_symbol) {
                    for tf in target_files {
                        if tf != &edge.rel_path {
                            *cross_file_deps
                                .entry((edge.rel_path.clone(), tf.clone()))
                                .or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    for rel_path in file_hashes.keys().take(500) {
        if let Ok(edges) = store.list_code_edges_from(&ws_key, rel_path, 50) {
            for edge in edges {
                if let Some(target_files) = sym_to_files.get(&edge.to_symbol) {
                    for tf in target_files {
                        if tf != rel_path {
                            *cross_file_deps
                                .entry((rel_path.clone(), tf.clone()))
                                .or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }

    let mut modules: Vec<Value> = dir_files
        .into_iter()
        .map(|(dir, files)| {
            let sym_count = dir_symbols.get(&dir).copied().unwrap_or(0);
            let langs: Vec<String> = dir_languages
                .remove(&dir)
                .map(|s| s.into_iter().collect())
                .unwrap_or_default();
            let kinds = dir_kinds.remove(&dir).unwrap_or_default();
            json!({
                "module": dir,
                "file_count": files.len(),
                "symbol_count": sym_count,
                "languages": langs,
                "kinds": kinds
            })
        })
        .collect();
    modules.sort_by(|a, b| {
        b["symbol_count"]
            .as_u64()
            .cmp(&a["symbol_count"].as_u64())
            .then_with(|| a["module"].as_str().cmp(&b["module"].as_str()))
    });
    modules.truncate(max_modules.clamp(1, 200));

    let mut dependencies: Vec<Value> = cross_file_deps
        .into_iter()
        .map(|((from_file, to_file), count)| {
            json!({
                "from_file": from_file,
                "to_file": to_file,
                "reference_count": count
            })
        })
        .collect();
    dependencies.sort_by(|a, b| {
        b["reference_count"]
            .as_u64()
            .cmp(&a["reference_count"].as_u64())
    });
    dependencies.truncate(100);

    Ok(json!({
        "workspace_root": workspace_root.display().to_string(),
        "total_files": stats.file_count,
        "total_symbols": stats.symbol_count,
        "total_edges": stats.edge_count,
        "modules": modules,
        "cross_file_dependencies": dependencies
    }))
}

/// Report indexed files, symbols, edges, and language breakdown via `code_index_stats`.
pub fn codebase_coverage(store: &Store, workspace_root: &Path) -> Result<Value, String> {
    let ws_key = ensure_indexed(store, workspace_root)?;
    let stats = store
        .code_index_stats(&ws_key)
        .map_err(|e| e.to_string())?;
    let file_hashes = store
        .get_code_file_hashes(&ws_key)
        .map_err(|e| e.to_string())?;
    let symbols = store
        .search_code_symbols(&ws_key, "", None, 50_000)
        .map_err(|e| e.to_string())?;

    let mut files_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut syms_by_lang: BTreeMap<String, usize> = BTreeMap::new();
    let mut syms_by_kind: BTreeMap<String, usize> = BTreeMap::new();

    for rel_path in file_hashes.keys() {
        let lang = detect_language_from_path(Path::new(rel_path)).unwrap_or("unknown");
        *files_by_lang.entry(lang.to_string()).or_insert(0) += 1;
    }
    for sym in &symbols {
        *syms_by_lang.entry(sym.language.clone()).or_insert(0) += 1;
        *syms_by_kind.entry(sym.kind.clone()).or_insert(0) += 1;
    }

    let ast_engine = if cfg!(not(any(target_os = "android", target_os = "ios"))) {
        "tree-sitter"
    } else {
        "regex-fallback"
    };

    Ok(json!({
        "workspace_root": workspace_root.display().to_string(),
        "ast_engine": ast_engine,
        "indexed_files": stats.file_count,
        "indexed_symbols": stats.symbol_count,
        "indexed_edges": stats.edge_count,
        "last_updated_at_ms": stats.last_updated_at_ms,
        "files_by_language": files_by_lang,
        "symbols_by_language": syms_by_lang,
        "symbols_by_kind": syms_by_kind
    }))
}

fn resolve_workspace_root<'a>(
    ctx: &'a ToolContext,
    fallback: &'a Option<PathBuf>,
) -> &'a Path {
    if ctx.workspace_root.exists() {
        &ctx.workspace_root
    } else if let Some(ref fb) = fallback {
        fb.as_path()
    } else {
        &ctx.workspace_root
    }
}

/// Register all 7 Deferred Code-Memory tools directly from an `Arc<Store>`.
pub fn register_code_memory_tools(registry: &mut ToolRegistry, store: Arc<Store>) {
    register_code_memory_tools_internal(registry, store, None);
}

/// Register all 7 Deferred Code-Memory tools on `ToolRegistry` using a `ModulesSuite`.
pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    register_code_memory_tools_internal(
        registry,
        Arc::new(suite.store.clone()),
        Some(suite.workspace_root.clone()),
    );
}

fn register_code_memory_tools_internal(
    registry: &mut ToolRegistry,
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
) {
    registry.register(Arc::new(CodebaseIndexTool {
        store: Arc::clone(&store),
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseSearchTool {
        store: Arc::clone(&store),
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseTextSearchTool {
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseReadSymbolTool {
        store: Arc::clone(&store),
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseTraceTool {
        store: Arc::clone(&store),
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseArchitectureTool {
        store: Arc::clone(&store),
        fallback_workspace: fallback_workspace.clone(),
    }));
    registry.register(Arc::new(CodebaseCoverageTool {
        store,
        fallback_workspace,
    }));
}

struct CodebaseIndexTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseIndexTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_index".into(),
                description: "Index or incrementally update workspace source symbols & call graph into SQLite Code-Memory using Tree-sitter AST.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "force_full": { "type": "boolean", "description": "Re-parse all files ignoring cached SHA-256 hashes (default false)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let force_full = args
            .get("force_full")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match index_workspace(&self.store, root, force_full) {
            Ok(stats) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&stats).unwrap_or(json!({})),
                format!(
                    "Indexed {} files ({} symbols, {} reused) in {}ms",
                    stats.total_files, stats.total_symbols, stats.files_reused, stats.duration_ms
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("codebase_index_error", e, None),
        }
    }
}

struct CodebaseSearchTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_search".into(),
                description: "Search indexed workspace AST symbols via FTS5 by name, kind, or path with ranked matches.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Symbol name or substring" },
                        "kind": { "type": "string", "description": "Optional symbol kind (function, method, struct, class, enum, trait, interface, type, module, constant)" },
                        "path": { "type": "string", "description": "Optional relative path substring filter" },
                        "limit": { "type": "integer", "description": "Maximum matches (default 20)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let kind = args.get("kind").and_then(|v| v.as_str());
        let path_filter = args.get("path").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match search_symbols(&self.store, root, query, kind, path_filter, limit) {
            Ok(matches) => {
                let count = matches.len();
                let files: BTreeSet<&str> = matches
                    .iter()
                    .filter_map(|m| m.get("file").and_then(|v| v.as_str()))
                    .collect();
                ToolExecutionResult::ok_structured(
                    json!({
                        "query": query,
                        "count": count,
                        "files_matched": files.len(),
                        "matches": matches
                    }),
                    format!("Found {count} symbol match(es) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_search_error", e, None),
        }
    }
}

struct CodebaseTextSearchTool {
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseTextSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_text_search".into(),
                description: "Fast bounded literal or regex text search across indexed workspace source files.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "regex": { "type": "boolean", "description": "Treat query as regex (default false)" },
                        "path": { "type": "string", "description": "Optional relative path substring filter" },
                        "limit": { "type": "integer", "description": "Maximum matches (default 30)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let is_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
        let path_filter = args.get("path").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match search_text(root, query, is_regex, path_filter, limit) {
            Ok(matches) => {
                let count = matches.len();
                ToolExecutionResult::ok_structured(
                    json!({
                        "query": query,
                        "regex": is_regex,
                        "count": count,
                        "matches": matches
                    }),
                    format!("Found {count} text match(es) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_text_search_error", e, None),
        }
    }
}

struct CodebaseReadSymbolTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseReadSymbolTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_read_symbol".into(),
                description: "Read the exact source snippet around a symbol's file and line range."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Symbol name to resolve in the index" },
                        "file": { "type": "string", "description": "Optional relative file path" },
                        "line": { "type": "integer", "description": "Optional 1-based line number" },
                        "context_lines": { "type": "integer", "description": "Lines of snippet context (default 30)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let name = args.get("name").and_then(|v| v.as_str());
        let file = args.get("file").and_then(|v| v.as_str());
        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(30) as usize;
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match read_symbol_snippet(&self.store, root, name, file, line, context_lines) {
            Ok(data) => {
                let target_file = data["file"].as_str().unwrap_or("").to_string();
                let target_line = data["target_line"].as_u64().unwrap_or(1);
                ToolExecutionResult::ok_structured(
                    data,
                    format!("Read symbol snippet at {target_file}:{target_line}"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_read_symbol_error", e, None),
        }
    }
}

struct CodebaseTraceTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseTraceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_trace".into(),
                description: "Trace callers and/or callees of a symbol across the indexed AST call graph.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Symbol name to trace" },
                        "direction": { "type": "string", "enum": ["callers", "callees", "both"], "description": "Traversal direction (default 'both')" },
                        "depth": { "type": "integer", "description": "Traversal depth 1..=4 (default 2)" },
                        "limit": { "type": "integer", "description": "Max edges per direction (default 50)" }
                    },
                    "required": ["symbol"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
        let direction = args
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("both");
        let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match trace_symbol(&self.store, root, symbol, direction, depth, limit) {
            Ok(data) => {
                let callers = data["caller_count"].as_u64().unwrap_or(0);
                let callees = data["callee_count"].as_u64().unwrap_or(0);
                ToolExecutionResult::ok_structured(
                    data,
                    format!(
                        "Traced '{symbol}' ({direction}, depth {}): {callers} caller(s), {callees} callee(s)",
                        depth.clamp(1, 4)
                    ),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_trace_error", e, None),
        }
    }
}

struct CodebaseArchitectureTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseArchitectureTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_architecture".into(),
                description: "Summarize directory/module symbol distribution, languages, and cross-file dependencies.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "max_modules": { "type": "integer", "description": "Maximum modules to return (default 50)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let max_modules = args
            .get("max_modules")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match codebase_architecture(&self.store, root, max_modules) {
            Ok(data) => {
                let mods = data["modules"].as_array().map(|a| a.len()).unwrap_or(0);
                let deps = data["cross_file_dependencies"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                ToolExecutionResult::ok_structured(
                    data,
                    format!("Architecture summary: {mods} module(s), {deps} cross-file dep(s)"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_architecture_error", e, None),
        }
    }
}

struct CodebaseCoverageTool {
    store: Arc<Store>,
    fallback_workspace: Option<PathBuf>,
}

impl ToolHandler for CodebaseCoverageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_coverage".into(),
                description: "Report indexed file, symbol, edge counts and language breakdown from SQLite Code-Memory.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, _args: Value) -> ToolExecutionResult {
        let root = resolve_workspace_root(ctx, &self.fallback_workspace);
        match codebase_coverage(&self.store, root) {
            Ok(data) => {
                let files = data["indexed_files"].as_u64().unwrap_or(0);
                let syms = data["indexed_symbols"].as_u64().unwrap_or(0);
                let edges = data["indexed_edges"].as_u64().unwrap_or(0);
                ToolExecutionResult::ok_structured(
                    data,
                    format!("Code-Memory coverage: {files} file(s), {syms} symbol(s), {edges} edge(s)"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_coverage_error", e, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_memory_indexes_searches_traces_and_summarizes_rust_ts_python_project() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let py_dir = dir.path().join("scripts");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&py_dir).unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/ignored.ts"),
            "export function shouldNeverBeIndexed() {}\n",
        )
        .unwrap();
        fs::write(
            src.join("engine.rs"),
            r#"pub struct QuantumPlanner {
    pub depth: usize,
}

impl QuantumPlanner {
    pub fn plan_orbit(&self, step: usize) -> usize {
        compute_trajectory(step)
    }
}

pub fn compute_trajectory(step: usize) -> usize {
    scale_factor(step) * 42
}

pub fn scale_factor(n: usize) -> usize {
    n + 1
}
"#,
        )
        .unwrap();
        fs::write(
            src.join("widget.ts"),
            r#"export interface DashboardState {
    ready: boolean;
}

export function renderDashboard(state: DashboardState): boolean {
    return formatStatus(state.ready);
}

export const formatStatus = (ready: boolean): boolean => ready;
"#,
        )
        .unwrap();
        fs::write(
            py_dir.join("pipeline.py"),
            r#"class DataPipeline:
    def run_stage(self, x):
        return normalize_value(x)

def normalize_value(v):
    return v * 2
"#,
        )
        .unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut reg = ToolRegistry::with_builtin_tools();
        register_code_memory_tools(&mut reg, Arc::clone(&store));
        let ctx = ToolContext::new(dir.path());

        // 1. First index parses Rust, TypeScript, and Python via Tree-sitter AST and skips node_modules
        let idx1 = reg.execute("codebase_index", &ctx, json!({}));
        assert!(!idx1.is_error, "{}", idx1.payload.to_model_value());
        let idx1_v = idx1.payload.to_model_value();
        assert_eq!(idx1_v["total_files"], 3);
        assert_eq!(idx1_v["files_indexed"], 3);
        assert!(idx1_v["total_symbols"].as_u64().unwrap() >= 8);

        // 2. Incremental second index skips unchanged files by SHA-256
        let idx2 = reg.execute("codebase_index", &ctx, json!({}));
        assert!(!idx2.is_error);
        let idx2_v = idx2.payload.to_model_value();
        assert_eq!(idx2_v["files_indexed"], 0);
        assert_eq!(idx2_v["files_reused"], 3);

        // 3. FTS5 symbol search finds Rust, TS, and Python symbols and skips ignored dirs
        let sym_hit = reg.execute(
            "codebase_search",
            &ctx,
            json!({"query": "compute_trajectory", "kind": "function"}),
        );
        assert!(!sym_hit.is_error);
        let sym_v = sym_hit.payload.to_model_value();
        assert_eq!(sym_v["count"], 1);
        assert_eq!(sym_v["matches"][0]["file"], "src/engine.rs");
        assert_eq!(sym_v["matches"][0]["language"], "rust");

        let py_hit = reg.execute(
            "codebase_search",
            &ctx,
            json!({"query": "DataPipeline", "kind": "class"}),
        );
        assert!(!py_hit.is_error);
        assert_eq!(py_hit.payload.to_model_value()["count"], 1);
        assert_eq!(
            py_hit.payload.to_model_value()["matches"][0]["language"],
            "python"
        );

        let ignored_hit = reg.execute(
            "codebase_search",
            &ctx,
            json!({"query": "shouldNeverBeIndexed"}),
        );
        assert_eq!(ignored_hit.payload.to_model_value()["count"], 0);

        // 4. Text search (literal & regex)
        let txt_hit = reg.execute(
            "codebase_text_search",
            &ctx,
            json!({"query": "scale_factor\\(step\\) \\* 42", "regex": true}),
        );
        assert!(!txt_hit.is_error);
        assert_eq!(txt_hit.payload.to_model_value()["count"], 1);

        // 5. Read symbol snippet
        let read_res = reg.execute(
            "codebase_read_symbol",
            &ctx,
            json!({"name": "renderDashboard", "context_lines": 10}),
        );
        assert!(!read_res.is_error, "{}", read_res.payload.to_model_value());
        let read_v = read_res.payload.to_model_value();
        assert_eq!(read_v["file"], "src/widget.ts");
        assert!(read_v["snippet"]
            .as_str()
            .unwrap()
            .contains("renderDashboard"));

        // 6. codebase_trace: callers & callees across AST call graph
        let trace_res = reg.execute(
            "codebase_trace",
            &ctx,
            json!({"symbol": "compute_trajectory", "direction": "both", "depth": 2}),
        );
        assert!(!trace_res.is_error, "{}", trace_res.payload.to_model_value());
        let trace_v = trace_res.payload.to_model_value();
        assert!(
            trace_v["callers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["from"] == "plan_orbit"),
            "expected plan_orbit caller: {trace_v}"
        );
        assert!(
            trace_v["callees"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["to"] == "scale_factor"),
            "expected scale_factor callee: {trace_v}"
        );

        // 7. codebase_architecture
        let arch_res = reg.execute("codebase_architecture", &ctx, json!({}));
        assert!(!arch_res.is_error, "{}", arch_res.payload.to_model_value());
        let arch_v = arch_res.payload.to_model_value();
        assert_eq!(arch_v["total_files"], 3);
        assert!(arch_v["modules"].as_array().unwrap().len() >= 2);

        // 8. codebase_coverage
        let cov_res = reg.execute("codebase_coverage", &ctx, json!({}));
        assert!(!cov_res.is_error, "{}", cov_res.payload.to_model_value());
        let cov_v = cov_res.payload.to_model_value();
        assert_eq!(cov_v["indexed_files"], 3);
        assert!(cov_v["indexed_symbols"].as_u64().unwrap() >= 8);
        assert!(cov_v["indexed_edges"].as_u64().unwrap() >= 3);
        assert_eq!(cov_v["files_by_language"]["rust"], 1);
        assert_eq!(cov_v["files_by_language"]["typescript"], 1);
        assert_eq!(cov_v["files_by_language"]["python"], 1);

        // 9. Deleting a file prunes it on next index
        fs::remove_file(py_dir.join("pipeline.py")).unwrap();
        let idx3 = reg.execute("codebase_index", &ctx, json!({}));
        assert!(!idx3.is_error);
        assert_eq!(idx3.payload.to_model_value()["total_files"], 2);
        let pruned_hit = reg.execute("codebase_search", &ctx, json!({"query": "DataPipeline"}));
        assert_eq!(pruned_hit.payload.to_model_value()["count"], 0);
    }

    #[test]
    fn code_memory_handles_deeply_nested_ast_skips_symlinks_and_derived_data() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();

        // Deeply nested expression tree that would overflow the call stack under recursive visit_node
        let depth = 1_200usize;
        let mut deep_rs = String::from("pub fn deep_chain(x: usize) -> usize {\n    ");
        for _ in 0..depth {
            deep_rs.push_str("(");
        }
        deep_rs.push_str("x");
        for _ in 0..depth {
            deep_rs.push_str(" + 1)");
        }
        deep_rs.push_str("\n}\n");
        fs::write(src.join("deep.rs"), &deep_rs).unwrap();

        // DerivedData and .gradle must be skipped
        let derived = dir.path().join("DerivedData/Build");
        fs::create_dir_all(&derived).unwrap();
        fs::write(derived.join("generated.swift"), "func neverIndexDerived() {}\n").unwrap();
        let gradle = dir.path().join(".gradle/cache");
        fs::create_dir_all(&gradle).unwrap();
        fs::write(gradle.join("cached.kt"), "fun neverIndexGradle() {}\n").unwrap();

        // Symlink cycle must be skipped safely without infinite recursion
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(dir.path(), src.join("loop_link"));
        }

        let store = Store::open_in_memory().unwrap();
        let stats = index_workspace(&store, dir.path(), true).unwrap();
        assert_eq!(stats.total_files, 1);

        let arch = codebase_architecture(&store, dir.path(), 50).unwrap();
        assert_eq!(arch["total_files"], 1);
        let hits = search_symbols(&store, dir.path(), "deep_chain", Some("function"), None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["file"], "src/deep.rs");

        let derived_hits = search_symbols(&store, dir.path(), "neverIndexDerived", None, None, 10).unwrap();
        assert!(derived_hits.is_empty());
        let gradle_hits = search_symbols(&store, dir.path(), "neverIndexGradle", None, None, 10).unwrap();
        assert!(gradle_hits.is_empty());
    }
}
