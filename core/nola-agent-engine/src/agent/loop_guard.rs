// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Detects repeated model tool batches without imposing a turn limit.

use serde_json::Value;
use std::collections::BTreeSet;

use super::host::{HostLoopProgressDeclaration, HostResourceProjectionDeclaration};
use super::protocol::ModelToolCall;

pub const DEFAULT_REPEATED_TOOL_BATCH_LIMIT: u8 = 3;
pub const DELEGATED_OUTPUT_GUARD_MIN_CHARS: usize = 8 * 1024;
pub const DELEGATED_OUTPUT_GUARD_CHECK_INTERVAL_CHARS: usize = 4 * 1024;
pub const DELEGATED_OUTPUT_GUARD_MAX_CHARS: usize = 128 * 1024;

pub fn tool_batch_signature(calls: &[ModelToolCall]) -> Option<String> {
    if calls.is_empty() {
        return None;
    }
    let mut calls = calls
        .iter()
        .map(|call| format!("{}:{}", call.name, canonical_json(&call.arguments)))
        .collect::<Vec<_>>();
    // Core schedules every accepted batch from its frozen capability effects
    // and dependency rules. Provider ordering is therefore transport detail,
    // not evidence of new work. Canonicalize it so a model cannot evade the
    // repeated-batch guard by permuting the same independent calls.
    calls.sort_unstable();
    Some(calls.join("|"))
}

pub fn observe_repeated_tool_batch(
    calls: &[ModelToolCall],
    previous_signature: &mut Option<String>,
    repeated_count: &mut u8,
    limit: u8,
) -> bool {
    let Some(signature) = tool_batch_signature(calls) else {
        *previous_signature = None;
        *repeated_count = 0;
        return false;
    };
    if previous_signature.as_deref() == Some(signature.as_str()) {
        *repeated_count = repeated_count.saturating_add(1);
    } else {
        *previous_signature = Some(signature);
        *repeated_count = 1;
    }
    *repeated_count >= limit.max(2)
}

pub(crate) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_default(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

/// A structural identity for repeated investigation of the same resource.
///
/// The family deliberately keeps resource-bearing arguments while separating
/// selectors such as a search pattern or byte range. This lets Core notice a
/// model walking the same resource with cosmetically different selectors
/// without interpreting user-authored text or task-specific vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvestigationIdentity {
    pub family: String,
    pub resource: String,
    pub selector: String,
    pub covered_until: Option<i64>,
}

pub(crate) fn investigation_identity(
    tool_name: &str,
    arguments: &Value,
    declaration: &HostLoopProgressDeclaration,
) -> Option<InvestigationIdentity> {
    let object = arguments.as_object()?;
    let mut resources = Vec::new();
    let mut selectors = Vec::new();
    let mut range_start = None;
    let mut range_extent = None;

    for key in &declaration.resource_keys {
        if let Some(value) = object.get(key) {
            if let Some(value) = scalar_identity(value) {
                resources.push((key.to_ascii_lowercase(), value));
            }
        }
    }
    if let Some(resource) = declaration
        .constant_resource
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resources.push(("constant".to_string(), normalized_resource_value(resource)));
    }
    for key in &declaration.selector_keys {
        if let Some(value) = object.get(key) {
            if let Some(number) = value.as_i64() {
                if declaration
                    .range_start_keys
                    .iter()
                    .any(|candidate| candidate == key)
                {
                    range_start = Some(number);
                } else if declaration
                    .range_extent_keys
                    .iter()
                    .any(|candidate| candidate == key)
                {
                    range_extent = Some(number.max(0));
                }
            }
            selectors.push((key.to_ascii_lowercase(), canonical_json(value)));
        }
    }
    if resources.is_empty() || selectors.is_empty() {
        return None;
    }
    resources.sort();
    selectors.sort();
    let resource_key = resources
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let resource = resources
        .iter()
        .map(|(_, value)| value.clone())
        .collect::<Vec<_>>()
        .join("&");
    let selector = std::iter::once(format!("tool={}", tool_name.to_ascii_lowercase()))
        .chain(
            selectors
                .iter()
                .map(|(key, value)| format!("{key}={value}")),
        )
        .collect::<Vec<_>>()
        .join("&");
    let covered_until = range_start.map(|start| {
        range_extent
            .map(|extent| start.saturating_add(extent))
            .unwrap_or(start)
    });
    Some(InvestigationIdentity {
        // Different investigative tools can walk the same resource without
        // changing the evidence boundary (for example indexed-symbol search
        // followed by indexed-text search). Keep the tool in the selector,
        // while the resource alone defines the family, so switching tools
        // cannot reset a stalled investigation.
        family: resource_key,
        resource,
        selector,
        covered_until,
    })
}

/// Projects resource identities strictly from the frozen Host descriptor,
/// without capability- or argument-name heuristics.
pub(crate) fn declared_call_resources(
    arguments: &Value,
    declaration: &HostResourceProjectionDeclaration,
) -> BTreeSet<String> {
    let mut resources = BTreeSet::new();
    let Some(object) = arguments.as_object() else {
        return resources;
    };
    let base = declaration
        .base_key
        .as_deref()
        .and_then(|key| object.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let scopes = declaration
        .preferred_scope_keys
        .iter()
        .filter_map(|key| object.get(key))
        .collect::<Vec<_>>();
    if !scopes.is_empty() {
        for value in scopes {
            collect_declared_projection_resources(value, declaration, &mut resources, base, true);
        }
        return resources;
    }
    collect_declared_projection_resources(arguments, declaration, &mut resources, base, false);
    resources
}

pub(crate) fn declared_effect_resources(
    arguments: &Value,
    declaration: &HostResourceProjectionDeclaration,
    validation: bool,
) -> BTreeSet<String> {
    let keys = if validation { &declaration.validation_scope_keys } else { &declaration.mutation_scope_keys };
    if keys.is_empty() { return declared_call_resources(arguments, declaration); }
    let mut projection = declaration.clone();
    projection.preferred_scope_keys = keys.clone();
    // Explicit effect scopes are authoritative, including an absent scope.
    // Do not fall back to a different effect's fields or ambient cwd.
    projection.resource_collection_keys.clear();
    let selected = arguments.as_object().map(|object| {
        object.iter().filter(|(key, _)| keys.contains(key) || declaration.base_key.as_ref() == Some(key))
            .map(|(key, value)| (key.clone(), value.clone())).collect::<serde_json::Map<_, _>>()
    }).unwrap_or_default();
    declared_call_resources(&Value::Object(selected), &projection)
}

pub(crate) fn declared_mutation_is_ephemeral(
    arguments: &Value,
    declaration: &HostResourceProjectionDeclaration,
) -> bool {
    declaration
        .ephemeral_when_true_keys
        .iter()
        .any(|key| arguments.get(key).and_then(Value::as_bool) == Some(true))
        || declaration
            .durable_key
            .as_deref()
            .and_then(|key| arguments.get(key))
            .and_then(Value::as_bool)
            .is_some_and(|durable| !durable)
}

fn collect_declared_projection_resources(
    value: &Value,
    declaration: &HostResourceProjectionDeclaration,
    output: &mut BTreeSet<String>,
    base: Option<&str>,
    accept_scalar: bool,
) {
    if accept_scalar {
        if let Some(value) = scalar_identity(value) {
            output.insert(resolve_declared_resource(&value, base));
            return;
        }
    }
    if value.is_null() || value.is_boolean() || value.is_number() || value.is_string() {
        return;
    }
    if let Some(values) = value.as_array() {
        for value in values {
            collect_declared_projection_resources(value, declaration, output, base, accept_scalar);
        }
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, value) in object {
        let declared = declaration
            .resource_keys
            .iter()
            .any(|candidate| candidate == key)
            || declaration
                .resource_collection_keys
                .iter()
                .any(|candidate| candidate == key);
        if declared {
            collect_declared_projection_resources(value, declaration, output, base, true);
        } else if value.is_object() || value.is_array() {
            collect_declared_projection_resources(value, declaration, output, base, false);
        }
    }
}

pub(crate) fn resources_overlap(left: &str, right: &str) -> bool {
    let left = normalized_resource_value(left);
    let right = normalized_resource_value(right);
    same_resource_identity(&left, &right)
        || left == right
        || left
            .strip_suffix('/')
            .is_some_and(|prefix| right.starts_with(&format!("{prefix}/")))
        || right
            .strip_suffix('/')
            .is_some_and(|prefix| left.starts_with(&format!("{prefix}/")))
        || right.starts_with(&format!("{left}/"))
        || left.starts_with(&format!("{right}/"))
}

/// Returns whether a structured validation resource covers a target. Coverage
/// is directional: validating a directory covers a child file, while
/// validating one child does not prove the whole declared directory.
pub(crate) fn resource_covers(covering: &str, target: &str) -> bool {
    let covering = normalized_resource_value(covering);
    let target = normalized_resource_value(target);
    same_resource_identity(&covering, &target)
        || covering == target
        || target.starts_with(&format!("{}/", covering.trim_end_matches('/')))
}

/// Resource identities must already have the same explicit base. A matching
/// suffix cannot establish ownership across repositories or execution roots.
fn same_resource_identity(left: &str, right: &str) -> bool {
    left == right
}

/// Declared Bash scopes are interpreted relative to the command's cwd by the
/// Host before execution. Resolve that same structural boundary here so a
/// relative `output_artifacts` entry and an absolute `validation_targets`
/// entry refer to one resource in Core's engineering ledger. Keep opaque
/// wildcard/URI identities unchanged; they are not filesystem paths.
fn resolve_declared_resource(value: &str, base: Option<&str>) -> String {
    if value == "*"
        || base.is_none()
        || value.starts_with('/')
        || value.contains("://")
        || value.contains(':')
    {
        return value.to_string();
    }
    let base = base.expect("base checked above").trim_end_matches('/');
    if base.is_empty() {
        value.to_string()
    } else {
        normalized_resource_value(&format!("{base}/{value}"))
    }
}

fn scalar_identity(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(normalized_resource_value(value)),
        Value::Number(_) | Value::Bool(_) => Some(canonical_json(value)),
        _ => None,
    }
}

fn normalized_resource_value(value: &str) -> String {
    let mut components = Vec::new();
    let absolute = value.starts_with('/');
    let normalized_separators = value.replace('\\', "/");
    for component in normalized_separators.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.last().is_some_and(|part| *part != "..") {
                    components.pop();
                } else if !absolute {
                    components.push("..");
                }
            }
            component => components.push(component),
        }
    }
    let joined = components.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Detects model output that is expanding by repeating already-emitted prose.
///
/// This intentionally operates on output structure rather than language or
/// task keywords. Long source code and tables commonly repeat short tokens,
/// so only substantial lines/paragraphs participate and repetition must
/// dominate the sample before the guard trips.
pub fn output_is_structurally_repetitive(text: &str) -> bool {
    let units = text
        .split(|character| {
            matches!(
                character,
                '\n' | '\r' | '.' | '!' | '?' | '。' | '！' | '？'
            )
        })
        .map(normalize_output_unit)
        .filter(|unit| unit.chars().count() >= 24)
        .collect::<Vec<_>>();
    if units.len() >= 8 {
        let unique = units.iter().collect::<BTreeSet<_>>();
        if unique.len().saturating_mul(2) < units.len() {
            return true;
        }
    }

    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if normalized.chars().count() < DELEGATED_OUTPUT_GUARD_MIN_CHARS {
        return false;
    }

    // Fixed semantic windows catch paragraph-sized cycles even when a
    // provider changes delta or newline boundaries between repetitions.
    let chars = normalized.chars().collect::<Vec<_>>();
    let window = 160;
    let stride = 80;
    if chars.len() < window * 8 {
        return false;
    }
    let mut windows = Vec::new();
    let mut offset = 0;
    while offset + window <= chars.len() {
        windows.push(chars[offset..offset + window].iter().collect::<String>());
        offset += stride;
    }
    let unique = windows.iter().collect::<BTreeSet<_>>();
    unique.len().saturating_mul(3) < windows.len().saturating_mul(2)
}

fn normalize_output_unit(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(resources: &[&str], selectors: &[&str]) -> HostLoopProgressDeclaration {
        HostLoopProgressDeclaration {
            resource_keys: resources.iter().map(|value| value.to_string()).collect(),
            selector_keys: selectors.iter().map(|value| value.to_string()).collect(),
            ..HostLoopProgressDeclaration::default()
        }
    }

    fn resource_projection() -> HostResourceProjectionDeclaration {
        HostResourceProjectionDeclaration {
            resource_keys: vec!["path".to_string()],
            preferred_scope_keys: vec![
                "validation_targets".to_string(),
                "output_artifacts".to_string(),
            ],
            base_key: Some("cwd".to_string()),
            ..HostResourceProjectionDeclaration::default()
        }
    }

    #[test]
    fn repeated_batch_identity_is_independent_of_provider_call_order() {
        let call = |id: &str, name: &str, arguments: Value| ModelToolCall {
            call_id: super::super::protocol::CallId::from_string(id.to_string()),
            name: name.to_string(),
            arguments,
            permission_scope: None,
            permission_rationale: None,
        };
        let first = vec![
            call("one", "read_file", serde_json::json!({"path":"a.rs"})),
            call(
                "two",
                "grep",
                serde_json::json!({"path":"b.rs","pattern":"x"}),
            ),
        ];
        let reordered = vec![
            call(
                "later-two",
                "grep",
                serde_json::json!({"pattern":"x","path":"b.rs"}),
            ),
            call("later-one", "read_file", serde_json::json!({"path":"a.rs"})),
        ];

        assert_eq!(
            tool_batch_signature(&first),
            tool_batch_signature(&reordered)
        );
    }

    #[test]
    fn repeated_long_form_output_is_rejected_without_language_rules() {
        let repeated =
            "The same analysis is being reconsidered without new evidence or a changed result.\n"
                .repeat(180);
        assert!(output_is_structurally_repetitive(&repeated));
        assert!(!output_is_structurally_repetitive(
            "A short answer can legitimately repeat a label twice.\nA short answer can legitimately repeat a label twice."
        ));
    }

    #[test]
    fn resources_include_declared_outputs_and_validation_targets() {
        let mutation = declared_call_resources(
            &serde_json::json!({
                "cwd": "/workspace/tests",
                "output_artifacts": [
                    {"path":"output/primary.json", "operation":"create"},
                    {"path":"output/companion.txt", "operation":"edit"}
                ]
            }),
            &resource_projection(),
        );
        assert!(mutation.contains("/workspace/tests/output/primary.json"));
        assert!(mutation.contains("/workspace/tests/output/companion.txt"));

        let validation = declared_call_resources(
            &serde_json::json!({
                "cwd": "/workspace/tests",
                "validation_targets": [
                    "/workspace/tests/output/primary.json",
                    "/workspace/tests/output/companion.txt"
                ]
            }),
            &resource_projection(),
        );
        assert_eq!(mutation, validation);
    }

    #[test]
    fn relative_paths_cannot_borrow_an_unconfirmed_workspace_identity() {
        assert!(!resources_overlap(
            "agent-created.txt",
            "/workspace/project/agent-created.txt"
        ));
        assert!(!resource_covers(
            "/workspace/project/agent-created.txt",
            "agent-created.txt"
        ));
        assert!(!resources_overlap(
            "agent-created.txt",
            "nola://workspace/project/agent-created.txt"
        ));
        assert!(!resources_overlap(
            "agent-created.txt",
            "/workspace/project/other.txt"
        ));
    }

    #[test]
    fn repository_project_is_a_resource_across_code_intelligence_queries() {
        let search = investigation_identity(
            "codebase_search",
            &serde_json::json!({
                "project":"Nola",
                "query":"AgentEngineeringLoopState",
                "limit":20,
                "offset":0
            }),
            &progress(&["project"], &["query", "limit", "offset"]),
        )
        .expect("indexed project should identify a resource family");
        let symbol = investigation_identity(
            "codebase_read_symbol",
            &serde_json::json!({
                "project":"Nola",
                "qualified_name":"agent::turn::AgentEngineeringLoopState",
                "context_lines":12
            }),
            &progress(&["project", "qualified_name"], &["context_lines"]),
        )
        .expect("symbol read should identify a resource family");

        assert_eq!(search.resource, "Nola");
        assert_eq!(
            symbol.resource,
            "Nola&agent::turn::AgentEngineeringLoopState"
        );
        assert!(search.selector.contains("query="));
        assert!(symbol.selector.contains("tool=codebase_read_symbol"));
    }

    #[test]
    fn tool_discovery_queries_share_the_frozen_catalog_resource() {
        let first = investigation_identity(
            "tool_search",
            &serde_json::json!({"query":"capability-one", "max_results":5}),
            &HostLoopProgressDeclaration {
                constant_resource: Some("turn-capability-catalog".to_string()),
                selector_keys: vec!["query".to_string(), "max_results".to_string()],
                ..HostLoopProgressDeclaration::default()
            },
        )
        .expect("tool discovery has an implicit frozen catalog resource");
        let second = investigation_identity(
            "tool_search",
            &serde_json::json!({"query":"capability-two", "max_results":8}),
            &HostLoopProgressDeclaration {
                constant_resource: Some("turn-capability-catalog".to_string()),
                selector_keys: vec!["query".to_string(), "max_results".to_string()],
                ..HostLoopProgressDeclaration::default()
            },
        )
        .expect("tool discovery has an implicit frozen catalog resource");

        assert_eq!(first.family, second.family);
        assert_eq!(first.resource, "turn-capability-catalog");
        assert_ne!(first.selector, second.selector);
        assert!(investigation_identity(
            "unknown_selector_tool",
            &serde_json::json!({"query":"capability-three"}),
            &HostLoopProgressDeclaration::default(),
        )
        .is_none());
    }

    #[test]
    fn declared_scope_excludes_execution_control_directories() {
        let validation = declared_call_resources(
            &serde_json::json!({
                "cwd": "/workspace",
                "validation_targets": ["/workspace/output/primary.json"]
            }),
            &resource_projection(),
        );
        assert_eq!(
            validation,
            BTreeSet::from(["/workspace/output/primary.json".to_string()])
        );

        let mutation = declared_call_resources(
            &serde_json::json!({
                "cwd": "/workspace",
                "output_artifacts": [
                    {"path":"/workspace/output/primary.json", "operation":"create"}
                ]
            }),
            &resource_projection(),
        );
        assert_eq!(validation, mutation);
    }

    #[test]
    fn current_directory_observation_has_a_nonempty_resource_identity() {
        let declaration = HostResourceProjectionDeclaration {
            resource_keys: vec!["root".into()], ..Default::default()
        };
        for root in [".", "./", "./."] {
            assert_eq!(declared_call_resources(&serde_json::json!({"root":root}), &declaration),
                BTreeSet::from([".".to_string()]));
        }
        assert_eq!(declared_call_resources(&serde_json::json!({"root":".."}), &declaration),
            BTreeSet::from(["..".to_string()]));
    }

    #[test]
    fn resource_projection_ignores_undeclared_fields() {
        let resources = declared_call_resources(
            &serde_json::json!({
                "cwd": "/workspace",
                "path": "src/agent.rs",
                "source": "this is payload content, not a resource",
                "query": "also not a resource"
            }),
            &HostResourceProjectionDeclaration {
                resource_keys: vec!["path".to_string()],
                base_key: Some("cwd".to_string()),
                ..HostResourceProjectionDeclaration::default()
            },
        );

        assert_eq!(
            resources,
            BTreeSet::from(["/workspace/src/agent.rs".to_string()])
        );
    }

    #[test]
    fn ephemeral_mutation_uses_only_declared_contract_keys() {
        let declaration = HostResourceProjectionDeclaration {
            ephemeral_when_true_keys: vec!["dry_run".to_string()],
            durable_key: Some("durable".to_string()),
            ..HostResourceProjectionDeclaration::default()
        };

        assert!(!declared_mutation_is_ephemeral(
            &serde_json::json!({"preview": true}),
            &declaration,
        ));
        assert!(declared_mutation_is_ephemeral(
            &serde_json::json!({"dry_run": true}),
            &declaration,
        ));
        assert!(declared_mutation_is_ephemeral(
            &serde_json::json!({"durable": false}),
            &declaration,
        ));
    }
}
