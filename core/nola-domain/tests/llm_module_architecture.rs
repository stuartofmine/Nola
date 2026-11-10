// SPDX-License-Identifier: Apache-2.0
//! Syntax-level production dependency gate. Rust compilation remains required;
//! this does not claim to expand arbitrary macros or analyze the whole crate.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use syn::visit::Visit;
use syn::{Attribute, Item, Meta, UseTree};

fn requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let Ok(parts) = list.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return false;
            };
            if list.path.is_ident("all") {
                parts.iter().any(requires_test)
            } else {
                !parts.is_empty() && parts.iter().all(requires_test)
            }
        }
        _ => false,
    }
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && attr
                    .parse_args::<Meta>()
                    .is_ok_and(|meta| requires_test(&meta)))
    })
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(v) => &v.attrs,
        Item::Enum(v) => &v.attrs,
        Item::Fn(v) => &v.attrs,
        Item::Impl(v) => &v.attrs,
        Item::Mod(v) => &v.attrs,
        Item::Static(v) => &v.attrs,
        Item::Struct(v) => &v.attrs,
        Item::Trait(v) => &v.attrs,
        Item::Type(v) => &v.attrs,
        Item::Use(v) => &v.attrs,
        Item::Macro(v) => &v.attrs,
        Item::Union(v) => &v.attrs,
        Item::ForeignMod(v) => &v.attrs,
        Item::TraitAlias(v) => &v.attrs,
        _ => &[],
    }
}

struct Dependencies<'a> {
    modules: &'a BTreeSet<String>,
    edges: BTreeSet<String>,
    host_edges: BTreeSet<String>,
    violations: Vec<String>,
    inline_depth: usize,
}

impl Dependencies<'_> {
    fn path(&mut self, parts: &[String]) {
        if parts.first().is_some_and(|part| part == "crate") {
            if let Some(target) = parts.get(1) {
                self.host_edges.insert(target.clone());
            }
        }
        let target = if parts.first().is_some_and(|part| part == "super") {
            let parents = parts.iter().take_while(|part| *part == "super").count();
            if parents <= self.inline_depth {
                return;
            }
            if parents != self.inline_depth + 1 {
                self.violations
                    .push("parent traversal must use explicit crate paths".into());
                return;
            }
            parts.get(parents)
        } else if parts.starts_with(&["crate".into(), "llm".into()]) {
            parts.get(2)
        } else {
            return;
        };
        if let Some(target) = target.filter(|target| self.modules.contains(*target)) {
            self.edges.insert(target.clone());
        } else {
            self.violations.push(format!(
                "implementation bypasses explicit module interface: {}",
                parts.join("::")
            ));
        }
    }

    fn import(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.import(&path.tree, prefix);
                prefix.pop();
            }
            UseTree::Group(group) => {
                for tree in &group.items {
                    self.import(tree, prefix);
                }
            }
            UseTree::Name(name) => {
                let mut path = prefix.clone();
                path.push(name.ident.to_string());
                self.path(&path);
            }
            UseTree::Rename(rename) => {
                let mut path = prefix.clone();
                path.push(rename.ident.to_string());
                if path == ["crate"] || path == ["crate", "self"] {
                    self.violations
                        .push("crate-root aliases bypass explicit dependency paths".into());
                } else {
                    self.path(&path);
                }
            }
            UseTree::Glob(_) => self
                .violations
                .push("production wildcard import erases interface ownership".into()),
        }
    }
}

impl<'ast> Visit<'ast> for Dependencies<'_> {
    fn visit_visibility(&mut self, _: &'ast syn::Visibility) {
        // pub(super) restricts visibility; it is not a dependency on the parent.
    }
    fn visit_item(&mut self, item: &'ast Item) {
        if !test_only(item_attrs(item)) {
            syn::visit::visit_item(self, item);
        }
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.import(&item.tree, &mut Vec::new());
    }
    fn visit_item_extern_crate(&mut self, _: &'ast syn::ItemExternCrate) {
        self.violations
            .push("implementation modules must use explicit crate paths, not crate aliases".into());
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(
            &path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>(),
        );
        syn::visit::visit_path(self, path);
    }
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if let Some((_, items)) = &item.content {
            self.inline_depth += 1;
            for item in items {
                self.visit_item(item);
            }
            self.inline_depth -= 1;
        } else {
            self.violations.push(
                "production out-of-line modules must be registered in the LLM facade graph".into(),
            );
        }
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("include") {
            self.violations
                .push("textual include bypasses module privacy".into());
        }
        syn::visit::visit_macro(self, mac);
    }
}

fn dependencies(source: &str, modules: &BTreeSet<String>) -> (BTreeSet<String>, Vec<String>) {
    let (edges, _, violations) = dependencies_with_host(source, modules);
    (edges, violations)
}

fn dependencies_with_host(
    source: &str,
    modules: &BTreeSet<String>,
) -> (BTreeSet<String>, BTreeSet<String>, Vec<String>) {
    let parsed = syn::parse_file(source).expect("valid Rust source");
    let mut visitor = Dependencies {
        modules,
        edges: BTreeSet::new(),
        host_edges: BTreeSet::new(),
        violations: Vec::new(),
        inline_depth: 0,
    };
    visitor.visit_file(&parsed);
    (visitor.edges, visitor.host_edges, visitor.violations)
}

fn acyclic(
    graph: &BTreeMap<String, BTreeSet<String>>,
    node: &str,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> bool {
    if done.contains(node) {
        return true;
    }
    if !active.insert(node.into()) {
        return false;
    }
    for next in &graph[node] {
        if !acyclic(graph, next, active, done) {
            return false;
        }
    }
    active.remove(node);
    done.insert(node.into());
    true
}

#[test]
fn llm_production_modules_have_explicit_acyclic_dependencies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("apps/cli/src");
    let facade = syn::parse_file(&fs::read_to_string(root.join("llm.rs")).unwrap()).unwrap();
    let mut files = BTreeMap::new();
    for item in facade.items {
        let Item::Mod(module) = item else {
            continue;
        };
        if test_only(&module.attrs) {
            continue;
        }
        assert!(
            matches!(module.vis, syn::Visibility::Inherited),
            "LLM implementation modules must be private"
        );
        let path = module
            .attrs
            .iter()
            .find_map(|attr| {
                if !attr.path().is_ident("path") {
                    return None;
                }
                let Meta::NameValue(value) = &attr.meta else {
                    return None;
                };
                let syn::Expr::Lit(value) = &value.value else {
                    return None;
                };
                let syn::Lit::Str(path) = &value.lit else {
                    return None;
                };
                Some(path.value())
            })
            .expect("LLM module has an explicit source path");
        files.insert(module.ident.to_string(), root.join(path));
    }
    assert!(files.len() >= 3);
    let names = files.keys().cloned().collect();
    let mut graph = BTreeMap::new();
    for (name, file) in files {
        let (edges, host_edges, violations) =
            dependencies_with_host(&fs::read_to_string(file).unwrap(), &names);
        assert!(violations.is_empty(), "{name}: {violations:?}");
        for forbidden in ["interaction", "terminal_ui", "composer"] {
            assert!(
                !host_edges.contains(forbidden),
                "{name} must consume neutral execution context, not {forbidden}"
            );
        }
        if name == "provider_protocols" || name == "responses" {
            assert!(
                edges.iter().all(
                    |edge| ["contracts", "transport", "stream_state", "diagnostics"]
                        .contains(&edge.as_str())
                ),
                "{name} must depend only on neutral interfaces: {edges:?}"
            );
        }
        if name == "contracts" {
            assert!(
                edges.is_empty(),
                "contracts must not depend on LLM implementations"
            );
        }
        if ["transport", "stream_state", "diagnostics"].contains(&name.as_str()) {
            assert!(
                edges.iter().all(|edge| edge == "contracts"),
                "{name} is not a neutral dependency: {edges:?}"
            );
        }
        graph.insert(name, edges);
    }
    for name in graph.keys() {
        assert!(
            acyclic(&graph, name, &mut BTreeSet::new(), &mut BTreeSet::new()),
            "LLM module dependency cycle from {name}: {graph:?}"
        );
    }
}

#[test]
fn execution_context_has_no_host_module_dependency() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("apps/cli/src/execution_context.rs");
    let (edges, host_edges, errors) =
        dependencies_with_host(&fs::read_to_string(source).unwrap(), &BTreeSet::new());
    assert!(errors.is_empty(), "execution context: {errors:?}");
    assert!(edges.is_empty());
    assert!(
        host_edges.is_empty(),
        "execution context depends on Host modules: {host_edges:?}"
    );
}

#[test]
fn syntax_gate_detects_aliases_grouped_imports_and_qualified_calls() {
    let names = BTreeSet::from(["streaming".into(), "contracts".into(), "transport".into()]);
    assert!(dependencies("pub(super) struct Shared;", &names)
        .1
        .is_empty());
    for source in [
        "use super::streaming as renamed;",
        "use crate::{llm::{streaming::stream_chat as call}};",
        "fn f() { ::crate::llm::streaming::stream_chat(); }",
    ] {
        assert!(
            dependencies(source, &names).0.contains("streaming"),
            "{source}"
        );
    }
    for source in [
        "use super as facade;",
        "use crate::llm as facade;",
        "use super::{stream_chat as call};",
        "pub use crate::llm::stream_chat;",
        "fn f() { crate::llm::stream_chat(); }",
        "use super::contracts::*;",
        "use crate as root;",
        "use crate::{self as root};",
        "extern crate self as root;",
    ] {
        assert!(!dependencies(source, &names).1.is_empty(), "{source}");
    }
    let (edges, errors) = dependencies("#[cfg(test)] mod tests { use super::*; } use super::contracts::LlmConfig; #[cfg(not(test))] fn live() { super::streaming::stream_chat(); }", &names);
    assert!(errors.is_empty());
    assert_eq!(
        edges,
        BTreeSet::from(["contracts".into(), "streaming".into()])
    );
    let cyclic = BTreeMap::from([
        ("a".into(), BTreeSet::from(["b".into()])),
        ("b".into(), BTreeSet::from(["a".into()])),
    ]);
    assert!(!acyclic(
        &cyclic,
        "a",
        &mut BTreeSet::new(),
        &mut BTreeSet::new()
    ));
}

#[test]
fn syntax_gate_tracks_host_dependencies_in_imports_and_expressions() {
    for source in [
        "use crate::interaction::ExecutionCancellation;",
        "use crate::{interaction::{ExecutionCancellation as Token}};",
        "fn f() { crate::interaction::current_execution_id(); }",
    ] {
        let (_, host_edges, errors) = dependencies_with_host(source, &BTreeSet::new());
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert_eq!(host_edges, BTreeSet::from(["interaction".into()]));
    }
    let (_, edges, errors) = dependencies_with_host(
        "#[cfg(test)] mod tests { use crate::interaction::ExecutionCancellation; } use std::sync::Arc;",
        &BTreeSet::new(),
    );
    assert!(errors.is_empty());
    assert!(edges.is_empty());
}
