// SPDX-License-Identifier: Apache-2.0

use arborium::tree_sitter::{Node, Parser};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

thread_local! {
    /// Parsers are initialized only for languages encountered by this worker
    /// thread, then reused for the remaining files of that language.
    static PARSERS: RefCell<HashMap<&'static str, Parser>> = RefCell::new(HashMap::new());
}

/// Statically linked grammars. The graph schema exposes this exact registry so
/// callers never have to infer parser support from a file extension.
pub(crate) const SUPPORTED_GRAMMARS: &[&str] = &[
    "asciidoc",
    "asm",
    "awk",
    "bash",
    "batch",
    "c",
    "c-sharp",
    "cmake",
    "cpp",
    "css",
    "dart",
    "diff",
    "dockerfile",
    "dot",
    "elisp",
    "fish",
    "gitattributes",
    "glsl",
    "go",
    "graphql",
    "hcl",
    "hlsl",
    "html",
    "ini",
    "java",
    "javascript",
    "jinja2",
    "jq",
    "jsdoc",
    "json",
    "kotlin",
    "lua",
    "make",
    "markdown",
    "matlab",
    "meson",
    "ninja",
    "nix",
    "objc",
    "perl",
    "php",
    "powershell",
    "proto",
    "python",
    "query",
    "r",
    "regex",
    "ruby",
    "rust",
    "scss",
    "solidity",
    "sql",
    "ssh-config",
    "starlark",
    "svelte",
    "swift",
    "textproto",
    "toml",
    "tsx",
    "typescript",
    "verilog",
    "vim",
    "vue",
    "x86asm",
    "xml",
    "yaml",
    "zsh",
];

/// Languages whose declaration shapes have explicit symbol extraction rules.
/// Every supported grammar still participates in discovery and text search.
pub(crate) const SEMANTIC_LANGUAGES: &[&str] = &[
    "bash",
    "c",
    "c-sharp",
    "cpp",
    "dart",
    "go",
    "java",
    "javascript",
    "kotlin",
    "lua",
    "php",
    "python",
    "ruby",
    "rust",
    "solidity",
    "swift",
    "tsx",
    "typescript",
    "verilog",
];

#[derive(Clone, Debug)]
pub(crate) struct ExtractedSymbol {
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Extraction {
    pub language: &'static str,
    pub symbols: Vec<ExtractedSymbol>,
    pub references: Vec<ExtractedReference>,
    pub parse_issue: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ExtractedReference {
    pub source_qualified_name: String,
    pub kind: &'static str,
    pub target_name: String,
    pub source_line: usize,
}

pub(crate) fn supported_language(path: &Path) -> Option<&'static str> {
    let language = conventional_filename_language(path)
        .or_else(|| arborium::detect_language(path.to_string_lossy().as_ref()))?;
    grammar(language).map(|_| language)
}

fn conventional_filename_language(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    match name.as_str() {
        "cmakelists.txt" => Some("cmake"),
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "gnumakefile" | "makefile" => Some("make"),
        _ if name.starts_with("dockerfile.") || name.starts_with("containerfile.") => {
            Some("dockerfile")
        }
        _ if name.ends_with(".vh") || name.ends_with(".svh") => Some("verilog"),
        _ => None,
    }
}

fn grammar(language: &str) -> Option<arborium::tree_sitter::Language> {
    // Preserve the previously validated parsers for Nola's deepest semantic
    // profiles. Arborium supplies the long tail through the same in-process
    // tree-sitter ABI.
    match language {
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "kotlin" => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "swift" => Some(tree_sitter_swift::LANGUAGE.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        _ => arborium::get_language(language),
    }
}

pub(crate) fn extract(path: &Path, source: &[u8]) -> Result<Extraction, String> {
    let language_name =
        supported_language(path).ok_or_else(|| "unsupported_language".to_string())?;
    let tree = PARSERS.with(|cache| {
        let mut parsers = cache.borrow_mut();
        if !parsers.contains_key(language_name) {
            let language = grammar(language_name)
                .ok_or_else(|| format!("grammar_unavailable:{language_name}"))?;
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|error| format!("language_setup_failed:{error}"))?;
            parsers.insert(language_name, parser);
        }
        parsers
            .get_mut(language_name)
            .and_then(|parser| parser.parse(source, None))
            .ok_or_else(|| "parser_returned_no_tree".to_string())
    })?;
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    if SEMANTIC_LANGUAGES.contains(&language_name) {
        visit(
            tree.root_node(),
            source,
            path,
            &mut Vec::new(),
            0,
            &mut symbols,
            &mut references,
        );
    }
    symbols.sort_by(|left, right| {
        left.start_byte
            .cmp(&right.start_byte)
            .then(left.qualified_name.cmp(&right.qualified_name))
    });
    symbols.dedup_by(|left, right| {
        left.start_byte == right.start_byte && left.qualified_name == right.qualified_name
    });
    let parse_issue = tree
        .root_node()
        .has_error()
        .then(|| "tree_sitter_reported_error_or_missing_nodes".to_string());
    Ok(Extraction {
        language: language_name,
        symbols,
        references,
        parse_issue,
    })
}

fn visit(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    parents: &mut Vec<String>,
    type_depth: usize,
    output: &mut Vec<ExtractedSymbol>,
    references: &mut Vec<ExtractedReference>,
) {
    collect_reference(node, source, path, parents, references);
    let mut pushed = false;
    if let Some(symbol) = arrow_function_symbol(node, source, path, parents, type_depth) {
        if let Some(identity) = symbol.qualified_name.rsplit("::").next() {
            parents.push(identity.to_string());
            pushed = true;
        }
        output.push(symbol);
    }
    let kind = symbol_kind(node);
    let mut child_type_depth = type_depth;
    if node.kind() == "impl_item" {
        if let Some(type_node) = node.child_by_field_name("type") {
            if let Some(type_name) = declared_type_name(type_node, source) {
                parents.push(type_name);
                pushed = true;
                child_type_depth += 1;
            }
        }
    }
    if let Some(kind) = kind {
        if let Some(name_node) = declaration_name_node(node) {
            if let Ok(name) = name_node.utf8_text(source) {
                let name = name.trim();
                if !name.is_empty() {
                    let mut parts = Vec::with_capacity(parents.len() + 2);
                    parts.push(module_identity(path));
                    parts.extend(parents.iter().cloned());
                    let start = node.start_position();
                    let end = node.end_position();
                    let declared_kind = refined_symbol_kind(node, source, kind);
                    let effective_kind = if declared_kind == "Function" && type_depth > 0 {
                        "Method"
                    } else {
                        declared_kind
                    };
                    let signature = if matches!(effective_kind, "Function" | "Method") {
                        callable_signature(node, source)
                    } else {
                        first_line(node, source)
                    };
                    let identity = if matches!(effective_kind, "Function" | "Method") {
                        let digest = format!("{:x}", Sha256::digest(signature.as_bytes()));
                        format!("{name}/{}@{}", callable_arity(node), &digest[..12])
                    } else {
                        name.to_string()
                    };
                    parts.push(identity.clone());
                    output.push(ExtractedSymbol {
                        kind: effective_kind.to_string(),
                        name: name.to_string(),
                        qualified_name: parts.join("::"),
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        start_line: start.row + 1,
                        end_line: end.row + 1,
                        signature,
                    });
                    if is_container(node.kind()) {
                        parents.push(identity);
                        pushed = true;
                        if is_type_container(node.kind()) {
                            child_type_depth += 1;
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(
            child,
            source,
            path,
            parents,
            child_type_depth,
            output,
            references,
        );
    }
    if pushed {
        parents.pop();
    }
}

/// Arrow functions are anonymous in the grammar. A variable declarator gives
/// them a stable source-level name without inventing names for inline closures.
fn arrow_function_symbol(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    parents: &[String],
    type_depth: usize,
) -> Option<ExtractedSymbol> {
    if node.kind() != "variable_declarator" {
        return None;
    }
    let value = node.child_by_field_name("value")?;
    if value.kind() != "arrow_function" {
        return None;
    }
    let name_node = node.child_by_field_name("name")?;
    if !matches!(name_node.kind(), "identifier" | "property_identifier") {
        return None;
    }
    let name = name_node.utf8_text(source).ok()?.trim();
    if name.is_empty() {
        return None;
    }
    let signature = callable_signature(value, source);
    let digest = format!("{:x}", Sha256::digest(signature.as_bytes()));
    let identity = format!("{name}/{}@{}", callable_arity(value), &digest[..12]);
    let mut parts = Vec::with_capacity(parents.len() + 2);
    parts.push(module_identity(path));
    parts.extend(parents.iter().cloned());
    parts.push(identity);
    Some(ExtractedSymbol {
        kind: if type_depth > 0 { "Method" } else { "Function" }.to_string(),
        name: name.to_string(),
        qualified_name: parts.join("::"),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        signature,
    })
}

fn declared_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "type_identifier" | "identifier" | "simple_identifier"
    ) {
        return node
            .utf8_text(source)
            .ok()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = declared_type_name(child, source) {
            return Some(name);
        }
    }
    None
}

fn callable_arity(node: Node<'_>) -> usize {
    if node.child_by_field_name("parameter").is_some() {
        return 1;
    }
    let parameters = node.child_by_field_name("parameters").or_else(|| {
        let mut cursor = node.walk();
        let found = node.named_children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "parameters"
                    | "parameter_clause"
                    | "parameter_list"
                    | "formal_parameters"
                    | "formal_parameter_list"
                    | "method_parameters"
                    | "function_value_parameters"
                    | "value_parameters"
                    | "tf_port_list"
                    | "ParamDeclList"
            )
        });
        found
    });
    if let Some(parameters) = parameters {
        let mut cursor = parameters.walk();
        return parameters.named_children(&mut cursor).count();
    }
    let mut cursor = node.walk();
    let direct_parameters = node
        .named_children(&mut cursor)
        .filter(|child| {
            matches!(
                child.kind(),
                "parameter" | "lambda_parameter" | "required_parameter" | "optional_parameter"
            )
        })
        .count();
    direct_parameters
}

fn callable_signature(node: Node<'_>, source: &[u8]) -> String {
    let end = node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .or_else(|| {
            first_named_descendant_matching(
                node,
                &["function_statement_or_null", "statement_or_null"],
            )
            .map(|statement| statement.start_byte())
        })
        .unwrap_or_else(|| node.end_byte());
    String::from_utf8_lossy(&source[node.start_byte()..end])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1024)
        .collect()
}

fn symbol_kind(node: Node<'_>) -> Option<&'static str> {
    match node.kind() {
        "function_item"
        | "function_signature_item"
        | "function_definition"
        | "function_declaration"
        | "function_signature"
        | "local_function"
        | "function"
        | "task_declaration"
        | "task_prototype"
        | "FnProto" => {
            if node
                .parent()
                .is_some_and(|parent| parent.kind() == "method_signature")
            {
                None
            } else {
                Some("Function")
            }
        }
        "method_definition"
        | "method_declaration"
        | "method"
        | "method_signature"
        | "abstract_method_signature"
        | "function_declaration_with_body"
        | "protocol_function_declaration" => Some("Method"),
        "struct_item" | "struct_specifier" => Some("Struct"),
        "enum_item" | "enum_declaration" | "enum_specifier" => Some("Enum"),
        "trait_item" | "interface_declaration" | "interface_class_declaration" => Some("Interface"),
        "protocol_declaration" => Some("Interface"),
        "typealias_declaration" | "type_item" => Some("Type"),
        "class_definition"
        | "class_declaration"
        | "class_specifier"
        | "class"
        | "abstract_class_declaration"
        | "object_declaration"
        | "companion_object" => Some("Class"),
        "mod_item"
        | "module"
        | "module_declaration"
        | "package_declaration"
        | "program_declaration"
        | "namespace_definition" => Some("Module"),
        "type_alias_declaration" => Some("Type"),
        "type_declaration" | "data_type" | "union_specifier" => Some("Type"),
        "contract_declaration" => Some("Class"),
        "const_item" | "static_item" => Some("Constant"),
        "macro_definition" => Some("Macro"),
        _ => None,
    }
}

fn declaration_name_node(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(name) = verilog_declaration_name_node(node) {
        return Some(name);
    }
    if let Some(name) = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("function"))
    {
        return declaration_identifier(name).or(Some(name));
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        if let Some(name) = declaration_identifier(declarator) {
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
                | "name"
                | "constant"
                | "constructor"
                | "variable"
                | "word"
                | "IDENTIFIER"
        )
    });
    found
}

fn verilog_declaration_name_node(node: Node<'_>) -> Option<Node<'_>> {
    let identifier_kinds: &[&str] = match node.kind() {
        "module_declaration" => &["simple_identifier", "escaped_identifier"],
        "interface_declaration" => &["interface_identifier"],
        "package_declaration" => &["package_identifier"],
        "class_declaration" | "interface_class_declaration" => &["class_identifier"],
        "program_declaration" => &["program_identifier"],
        "function_declaration" => &["function_identifier", "method_identifier"],
        "task_declaration" | "task_prototype" => &["task_identifier", "method_identifier"],
        _ => return None,
    };
    first_named_descendant_matching(node, identifier_kinds)
        .and_then(|identifier| verilog_identifier_leaf(identifier).or(Some(identifier)))
}

fn first_named_descendant_matching<'tree>(
    node: Node<'tree>,
    kinds: &[&str],
) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return Some(child);
        }
        if let Some(found) = first_named_descendant_matching(child, kinds) {
            return Some(found);
        }
    }
    None
}

fn verilog_identifier_leaf(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "simple_identifier" | "escaped_identifier") {
        return Some(node);
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find_map(verilog_identifier_leaf);
    found
}

fn declaration_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        node.kind(),
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "simple_identifier"
            | "name"
            | "constant"
            | "constructor"
            | "variable"
            | "word"
            | "IDENTIFIER"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find_map(declaration_identifier);
    found
}

fn refined_symbol_kind<'a>(node: Node<'_>, source: &[u8], fallback: &'a str) -> &'a str {
    let Some(declaration_kind) = node.child_by_field_name("declaration_kind") else {
        return fallback;
    };
    match declaration_kind.utf8_text(source).ok().map(str::trim) {
        Some("struct") => "Struct",
        Some("enum") => "Enum",
        Some("protocol") => "Interface",
        _ => fallback,
    }
}

fn is_container(kind: &str) -> bool {
    matches!(
        kind,
        "struct_item"
            | "struct_specifier"
            | "enum_item"
            | "enum_declaration"
            | "enum_specifier"
            | "trait_item"
            | "class_definition"
            | "class_declaration"
            | "interface_class_declaration"
            | "class_specifier"
            | "class"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "protocol_declaration"
            | "object_declaration"
            | "companion_object"
            | "contract_declaration"
            | "union_specifier"
            | "mod_item"
            | "module"
            | "module_declaration"
            | "package_declaration"
            | "program_declaration"
            | "namespace_definition"
            | "function_item"
            | "function_definition"
            | "function_declaration"
            | "task_declaration"
            | "function"
            | "FnProto"
            | "local_function"
            | "method_definition"
            | "method_declaration"
            | "method"
            | "function_declaration_with_body"
    )
}

fn is_type_container(kind: &str) -> bool {
    matches!(
        kind,
        "struct_item"
            | "struct_specifier"
            | "enum_item"
            | "enum_declaration"
            | "enum_specifier"
            | "trait_item"
            | "class_definition"
            | "class_declaration"
            | "interface_class_declaration"
            | "class_specifier"
            | "class"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "protocol_declaration"
            | "object_declaration"
            | "companion_object"
            | "contract_declaration"
            | "union_specifier"
    )
}

fn collect_reference(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    parents: &[String],
    output: &mut Vec<ExtractedReference>,
) {
    let source_qualified_name = || {
        let mut parts = Vec::with_capacity(parents.len() + 1);
        parts.push(module_identity(path));
        parts.extend(parents.iter().cloned());
        parts.join("::")
    };
    if matches!(
        node.kind(),
        "call"
            | "call_expression"
            | "function_call"
            | "method_invocation"
            | "invocation_expression"
            | "command"
    ) {
        let target = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("callee"))
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("target"));
        if let Some(target) = target {
            if let Ok(text) = target.utf8_text(source) {
                if let Some(name) = terminal_identifier(text) {
                    output.push(ExtractedReference {
                        source_qualified_name: source_qualified_name(),
                        kind: "CALLS",
                        target_name: name,
                        source_line: node.start_position().row + 1,
                    });
                }
            }
        }
    }
    if matches!(
        node.kind(),
        "use_declaration"
            | "import_statement"
            | "import_from_statement"
            | "import_header"
            | "import_declaration"
            | "using_directive"
            | "include_directive"
    ) {
        for child in named_descendants(node) {
            if matches!(
                child.kind(),
                "identifier" | "type_identifier" | "simple_identifier" | "name"
            ) {
                if let Ok(name) = child.utf8_text(source) {
                    output.push(ExtractedReference {
                        source_qualified_name: module_identity(path),
                        kind: "IMPORTS",
                        target_name: name.to_string(),
                        source_line: child.start_position().row + 1,
                    });
                }
            }
        }
    }
    if matches!(
        node.kind(),
        "module_instantiation" | "checker_instantiation"
    ) {
        let mut cursor = node.walk();
        let target = node
            .named_children(&mut cursor)
            .find(|child| {
                matches!(
                    child.kind(),
                    "simple_identifier" | "escaped_identifier" | "checker_identifier"
                )
            })
            .and_then(|identifier| verilog_identifier_leaf(identifier).or(Some(identifier)));
        if let Some(target) = target {
            if let Ok(name) = target.utf8_text(source) {
                output.push(ExtractedReference {
                    source_qualified_name: source_qualified_name(),
                    kind: "INSTANTIATES",
                    target_name: name.trim().to_string(),
                    source_line: node.start_position().row + 1,
                });
            }
        }
    }
}

fn terminal_identifier(value: &str) -> Option<String> {
    value
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|part| !part.is_empty())
        .next_back()
        .map(ToOwned::to_owned)
}

fn named_descendants(node: Node<'_>) -> Vec<Node<'_>> {
    let mut result = Vec::new();
    let mut stack = vec![node];
    while let Some(parent) = stack.pop() {
        let mut child_cursor = parent.walk();
        for child in parent.named_children(&mut child_cursor) {
            result.push(child);
            stack.push(child);
        }
    }
    result
}

pub(crate) fn module_identity(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("::")
}

fn first_line(node: Node<'_>, source: &[u8]) -> String {
    let slice = &source[node.start_byte()..node.end_byte()];
    String::from_utf8_lossy(slice)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(512)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn symbols(path: &str, source: &str) -> Vec<ExtractedSymbol> {
        let extraction = extract(Path::new(path), source.as_bytes()).expect("source should parse");
        assert_eq!(extraction.parse_issue, None, "{path} should parse cleanly");
        extraction.symbols
    }

    fn symbol<'a>(symbols: &'a [ExtractedSymbol], name: &str, kind: &str) -> &'a ExtractedSymbol {
        symbols
            .iter()
            .find(|item| item.name == name && item.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind} {name}; got {symbols:#?}"))
    }

    #[test]
    fn every_declared_grammar_is_statically_available() {
        assert_eq!(SUPPORTED_GRAMMARS.len(), 67);
        assert_eq!(SUPPORTED_GRAMMARS.iter().collect::<HashSet<_>>().len(), 67);
        for language_name in SUPPORTED_GRAMMARS {
            let language = grammar(language_name)
                .unwrap_or_else(|| panic!("missing statically linked grammar {language_name}"));
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .unwrap_or_else(|error| panic!("invalid grammar {language_name}: {error}"));
            assert!(
                parser.parse(b"", None).is_some(),
                "grammar {language_name} returned no tree"
            );
        }
    }

    #[test]
    fn recognizes_conventional_source_filenames_without_extensions() {
        let fixtures = [
            ("Dockerfile", "dockerfile"),
            ("Containerfile.dev", "dockerfile"),
            ("Makefile", "make"),
            ("CMakeLists.txt", "cmake"),
        ];
        for (path, expected) in fixtures {
            assert_eq!(
                supported_language(Path::new(path)),
                Some(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn excludes_programming_languages_below_the_usage_floor() {
        for path in [
            "sample.cob",
            "sample.hs",
            "sample.lean",
            "sample.scala",
            "sample.zig",
        ] {
            assert_eq!(supported_language(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn recognizes_verilog_and_systemverilog_extensions() {
        for path in ["design.v", "design.vh", "design.sv", "design.svh"] {
            assert_eq!(
                supported_language(Path::new(path)),
                Some("verilog"),
                "{path}"
            );
        }
    }

    #[test]
    fn extracts_systemverilog_design_units_callables_and_instances() {
        let source = r#"
package common_types;
    function automatic int increment(input int value);
        return value + 1;
    endfunction
endpackage

interface stream_if(input logic clock);
    logic valid;
endinterface

interface class transaction_driver;
    pure virtual task drive(input int value);
endclass

class packet;
    function int size();
        return 1;
    endfunction
endclass

module worker(input logic clock);
endmodule

module controller(input logic clock);
    worker worker_instance(.clock(clock));

    function automatic int transform(input int value);
        return value;
    endfunction

    task drive(input int value);
    endtask
endmodule
"#;
        let extraction = extract(Path::new("rtl/controller.sv"), source.as_bytes())
            .expect("SystemVerilog source should parse");
        assert_eq!(extraction.language, "verilog");
        assert_eq!(extraction.parse_issue, None);

        let symbols = &extraction.symbols;
        symbol(symbols, "common_types", "Module");
        symbol(symbols, "stream_if", "Interface");
        symbol(symbols, "transaction_driver", "Interface");
        symbol(symbols, "packet", "Class");
        symbol(symbols, "worker", "Module");
        symbol(symbols, "controller", "Module");
        assert!(symbols.iter().any(|item| {
            item.kind == "Function"
                && item.name == "increment"
                && item.qualified_name.contains("::common_types::increment/")
        }));
        assert!(symbols.iter().any(|item| {
            item.kind == "Method"
                && item.name == "size"
                && item.qualified_name.contains("::packet::size/")
        }));
        assert!(symbols.iter().any(|item| {
            item.kind == "Method"
                && item.name == "drive"
                && item.qualified_name.contains("::transaction_driver::drive/")
        }));
        assert!(symbols.iter().any(|item| {
            item.kind == "Function"
                && item.name == "transform"
                && item.qualified_name.contains("::controller::transform/")
        }));
        assert!(symbols.iter().any(|item| {
            item.kind == "Function"
                && item.name == "drive"
                && item.qualified_name.contains("::controller::drive/")
        }));
        assert!(
            extraction.references.iter().any(|reference| {
                reference.kind == "INSTANTIATES"
                    && reference.target_name == "worker"
                    && reference.source_qualified_name.ends_with("::controller")
            }),
            "missing module instance reference; got {:#?}",
            extraction.references
        );
    }

    #[test]
    fn extracts_symbols_across_mainstream_language_families() {
        let fixtures = [
            (
                "main.go",
                "package p\ntype Store struct {}\nfunc Load(x int) int { return x }",
                "Load",
                "Function",
            ),
            (
                "Main.java",
                "class Store { int load(int x) { return x; } }",
                "load",
                "Method",
            ),
            (
                "main.c",
                "struct Store { int x; }; int load(int x) { return x; }",
                "load",
                "Function",
            ),
            (
                "main.cpp",
                "class Store { public: int load(int x) { return x; } };",
                "load",
                "Method",
            ),
            (
                "Main.cs",
                "class Store { int Load(int x) { return x; } }",
                "Load",
                "Method",
            ),
            (
                "main.rb",
                "class Store\n  def load(x)\n    x\n  end\nend",
                "load",
                "Method",
            ),
            (
                "main.php",
                "<?php class Store { function load($x) { return $x; } }",
                "load",
                "Method",
            ),
            ("main.sh", "load() { printf '%s' ok; }", "load", "Function"),
            (
                "main.lua",
                "function load(x) return x end",
                "load",
                "Function",
            ),
            ("main.dart", "int load(int x) => x;", "load", "Function"),
            (
                "main.sol",
                "contract Store { function load(uint x) public returns (uint) { return x; } }",
                "load",
                "Method",
            ),
        ];
        for (path, source, name, kind) in fixtures {
            let extracted = symbols(path, source);
            symbol(&extracted, name, kind);
        }
    }

    #[test]
    fn extracts_swift_type_kinds_and_scoped_methods() {
        let symbols = symbols(
            "Sources/Models.swift",
            r#"
struct Cache<T> {
    func load(key: String, fallback: T) -> T { fallback }
}
class Controller {
    func load(key: String) -> String { key }
}
protocol Loading {
    func load(key: String) -> String
}
"#,
        );

        assert_eq!(
            symbol(&symbols, "Cache", "Struct").qualified_name,
            "Sources::Models.swift::Cache"
        );
        assert_eq!(
            symbol(&symbols, "Controller", "Class").qualified_name,
            "Sources::Models.swift::Controller"
        );
        assert_eq!(
            symbol(&symbols, "Loading", "Interface").qualified_name,
            "Sources::Models.swift::Loading"
        );
        assert!(symbols.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("Sources::Models.swift::Cache::load/2@")));
        assert!(symbols.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("Sources::Models.swift::Controller::load/1@")));
        assert!(symbols.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("Sources::Models.swift::Loading::load/1@")));
    }

    #[test]
    fn extracts_kotlin_classes_objects_and_top_level_functions() {
        let symbols = symbols(
            "src/Worker.kt",
            r#"
class Worker {
    fun run(task: String): String = task
}
object Registry {
    fun run(task: String, retries: Int): String = task
}
fun run(task: String, retries: Int, urgent: Boolean): String = task
"#,
        );

        assert_eq!(
            symbol(&symbols, "Worker", "Class").qualified_name,
            "src::Worker.kt::Worker"
        );
        assert_eq!(
            symbol(&symbols, "Registry", "Class").qualified_name,
            "src::Worker.kt::Registry"
        );
        assert!(symbols.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("src::Worker.kt::Worker::run/1@")));
        assert!(symbols.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("src::Worker.kt::Registry::run/2@")));
        assert!(symbols.iter().any(|item| item.kind == "Function"
            && item.qualified_name.starts_with("src::Worker.kt::run/3@")));
    }

    #[test]
    fn extracts_javascript_and_typescript_named_arrow_functions() {
        let javascript = symbols(
            "web/actions.js",
            "const transform = (value, options) => options.apply(value);",
        );
        assert!(javascript.iter().any(|item| item.kind == "Function"
            && item
                .qualified_name
                .starts_with("web::actions.js::transform/2@")));

        let typescript = symbols(
            "web/actions.ts",
            r#"
interface Transformer {
    transform(value: string, options?: object): string;
}
const transform = (value: string): string => value;
"#,
        );
        assert!(typescript.iter().any(|item| item.kind == "Method"
            && item
                .qualified_name
                .starts_with("web::actions.ts::Transformer::transform/2@")));
        assert!(typescript.iter().any(|item| item.kind == "Function"
            && item
                .qualified_name
                .starts_with("web::actions.ts::transform/1@")));
    }

    #[test]
    fn keeps_rust_trait_and_generic_impl_method_scopes_distinct() {
        let symbols = symbols(
            "src/store.rs",
            r#"
trait Store<T> {
    fn get(&self, key: &str) -> Option<&T>;
}
struct Memory<T>(T);
impl<T> Store<T> for Memory<T> {
    fn get(&self, key: &str) -> Option<&T> { Some(&self.0) }
}
"#,
        );

        let trait_method = symbols
            .iter()
            .find(|item| {
                item.kind == "Method"
                    && item
                        .qualified_name
                        .starts_with("src::store.rs::Store::get/2@")
            })
            .expect("trait method");
        let impl_method = symbols
            .iter()
            .find(|item| {
                item.kind == "Method"
                    && item
                        .qualified_name
                        .starts_with("src::store.rs::Memory::get/2@")
            })
            .expect("generic impl method");
        assert_ne!(trait_method.qualified_name, impl_method.qualified_name);
    }
}
