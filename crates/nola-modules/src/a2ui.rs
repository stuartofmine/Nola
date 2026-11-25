// SPDX-License-Identifier: Apache-2.0
//! Pure-v2 Authoritative A2UI (Generative UI) Projection Engine & Tools.
//!
//! Produces exact `UiSnapshot` / `UiNodeSnapshot` / `UiDraftPreview` / `UiSurfaceUpdate`
//! schemas consumed by Web (`vendor/a2ui-svelte`), Android (`GenerativeUiSurface.kt`),
//! and Apple (`GenerativeUIProjection.swift`).

use std::collections::BTreeMap;
use std::sync::Arc;

use nola_store::Store;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::ModulesSuite;

pub const KIND_UI_SURFACE: &str = "ui_surface";
pub const DEFAULT_CATALOG_ID: &str = "nola.a2ui.basic.v1";

pub const MAX_ACTIVE_SURFACES_PER_CONVERSATION: usize = 8;
pub const MAX_NODES_PER_SURFACE: usize = 1_000;
pub const MAX_SURFACE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_SUMMARY_BYTES: usize = 4_096;
pub const MAX_INTERACTION_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_STORED_REVISIONS: usize = 20;

const SUPPORTED_COMPONENTS: &[&str] = &[
    "Text",
    "Row",
    "Column",
    "List",
    "Card",
    "Tabs",
    "Modal",
    "Divider",
    "Button",
    "TextField",
    "CheckBox",
    "ChoicePicker",
    "Slider",
    "DateTimeInput",
    "Icon",
];

pub fn catalog_digest() -> String {
    sha256_hex(format!("{DEFAULT_CATALOG_ID}:{}", SUPPORTED_COMPONENTS.join(",")).as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceOwner {
    pub authority_id: String,
    pub conversation_id: String,
    pub participant_id: String,
    pub context_id: String,
    pub originating_run_id: String,
    pub message_id: String,
    pub part_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceIdentity {
    pub owner: UiSurfaceOwner,
    pub surface_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiActionRef {
    pub instance_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNodeSnapshot {
    pub instance_id: String,
    pub component_id: String,
    pub component: String,
    pub props: Map<String, Value>,
    pub slots: BTreeMap<String, Vec<String>>,
    pub bindings: BTreeMap<String, String>,
    pub actions: BTreeMap<String, UiActionRef>,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
    pub catalog_id: String,
    pub catalog_digest: String,
    pub phase: String,
    pub root_instance_id: Option<String>,
    pub nodes: BTreeMap<String, UiNodeSnapshot>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEdit {
    Set {
        instance_id: String,
        binding: String,
        value: Value,
    },
    Delete {
        instance_id: String,
        binding: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiInteraction {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    #[serde(default)]
    pub edits: Vec<UiEdit>,
    pub action: Option<UiActionRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDraftPreviewInput {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    #[serde(default)]
    pub edits: Vec<UiEdit>,
    pub draft_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDraftPreview {
    pub identity: UiSurfaceIdentity,
    pub base_revision: u64,
    pub draft_generation: u64,
    pub snapshot: UiSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSurfaceRevisionRef {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredUiSurface {
    pub storage_key: String,
    pub conversation_id: String,
    pub surface_id: String,
    pub current: UiSnapshot,
    pub data_model: Value,
    pub revisions: BTreeMap<u64, UiSnapshot>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedInteraction {
    pub snapshot: UiSnapshot,
    pub data_model: Value,
    pub action_id: Option<String>,
    pub summary: String,
}

pub fn surface_storage_key(conversation_id: &str, surface_id: &str) -> String {
    format!("{conversation_id}::{surface_id}")
}

fn json_pointer_get<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() || pointer == "/" {
        return Some(root);
    }
    let normalized = if pointer.starts_with('/') {
        pointer.to_string()
    } else {
        format!("/{pointer}")
    };
    root.pointer(&normalized)
}

fn json_pointer_set(root: &mut Value, pointer: &str, val: Value) {
    let trimmed = pointer.trim_start_matches('/');
    if trimmed.is_empty() {
        *root = val;
        return;
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    if !root.is_object() {
        *root = json!({});
    }
    let mut cursor = root;
    for seg in &segments[..segments.len() - 1] {
        let key = seg.replace("~1", "/").replace("~0", "~");
        if !cursor.is_object() {
            *cursor = json!({});
        }
        let obj = cursor.as_object_mut().unwrap();
        cursor = obj.entry(key).or_insert_with(|| json!({}));
    }
    let last_key = segments
        .last()
        .unwrap()
        .replace("~1", "/")
        .replace("~0", "~");
    if !cursor.is_object() {
        *cursor = json!({});
    }
    cursor.as_object_mut().unwrap().insert(last_key, val);
}

fn json_pointer_delete(root: &mut Value, pointer: &str) {
    let trimmed = pointer.trim_start_matches('/');
    if trimmed.is_empty() {
        return;
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    let mut cursor = root;
    for seg in &segments[..segments.len() - 1] {
        let key = seg.replace("~1", "/").replace("~0", "~");
        let Some(next) = cursor.as_object_mut().and_then(|o| o.get_mut(&key)) else {
            return;
        };
        cursor = next;
    }
    let last_key = segments
        .last()
        .unwrap()
        .replace("~1", "/")
        .replace("~0", "~");
    if let Some(obj) = cursor.as_object_mut() {
        obj.remove(&last_key);
    }
}

/// Evaluate bindings from `data_model` onto every node and compute authoritative validation errors.
pub fn project_nodes(
    base_nodes: &BTreeMap<String, UiNodeSnapshot>,
    data_model: &Value,
) -> BTreeMap<String, UiNodeSnapshot> {
    let mut out = BTreeMap::new();
    for (id, node) in base_nodes {
        let mut projected = node.clone();
        for (binding_name, pointer) in &projected.bindings {
            if let Some(bound_val) = json_pointer_get(data_model, pointer) {
                projected.props.insert(binding_name.clone(), bound_val.clone());
            }
        }
        projected.validation_errors = validate_node(&projected);
        out.insert(id.clone(), projected);
    }
    out
}

fn validate_node(node: &UiNodeSnapshot) -> Vec<String> {
    let mut errors = Vec::new();
    let val = node.props.get("value");
    let required = node
        .props
        .get("required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let label = node
        .props
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or(&node.component_id);

    if required {
        let empty = match val {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(Value::Array(a)) => a.is_empty(),
            Some(Value::Bool(b)) => !*b,
            _ => false,
        };
        if empty {
            errors.push(format!("{label} is required"));
        }
    }

    if let Some(Value::String(s)) = val {
        if let Some(min_len) = node.props.get("minLength").and_then(|v| v.as_u64()) {
            if !s.is_empty() && (s.chars().count() as u64) < min_len {
                errors.push(format!("{label} must be at least {min_len} characters"));
            }
        }
        if let Some(max_len) = node.props.get("maxLength").and_then(|v| v.as_u64()) {
            if (s.chars().count() as u64) > max_len {
                errors.push(format!("{label} must be at most {max_len} characters"));
            }
        }
    }

    if let Some(num) = val.and_then(|v| v.as_f64()) {
        if let Some(min_v) = node.props.get("min").and_then(|v| v.as_f64()) {
            if num < min_v {
                errors.push(format!("{label} must be at least {min_v}"));
            }
        }
        if let Some(max_v) = node.props.get("max").and_then(|v| v.as_f64()) {
            if num > max_v {
                errors.push(format!("{label} must be at most {max_v}"));
            }
        }
    }

    errors
}

fn apply_edits_to_model(
    nodes: &BTreeMap<String, UiNodeSnapshot>,
    data_model: &mut Value,
    edits: &[UiEdit],
) -> Result<(), String> {
    for edit in edits {
        match edit {
            UiEdit::Set {
                instance_id,
                binding,
                value,
            } => {
                let node = nodes
                    .get(instance_id)
                    .ok_or_else(|| format!("unknown UI node '{instance_id}'"))?;
                if node.props.get("disabled") == Some(&Value::Bool(true)) {
                    return Err(format!("UI node '{instance_id}' is disabled"));
                }
                let pointer = node
                    .bindings
                    .get(binding)
                    .ok_or_else(|| format!("node '{instance_id}' has no binding '{binding}'"))?;
                json_pointer_set(data_model, pointer, value.clone());
            }
            UiEdit::Delete {
                instance_id,
                binding,
            } => {
                let node = nodes
                    .get(instance_id)
                    .ok_or_else(|| format!("unknown UI node '{instance_id}'"))?;
                if node.props.get("disabled") == Some(&Value::Bool(true)) {
                    return Err(format!("UI node '{instance_id}' is disabled"));
                }
                let pointer = node
                    .bindings
                    .get(binding)
                    .ok_or_else(|| format!("node '{instance_id}' has no binding '{binding}'"))?;
                json_pointer_delete(data_model, pointer);
            }
        }
    }
    Ok(())
}

/// Convert a snapshot into a strictly immutable historical read-only snapshot (no actions/bindings).
pub fn to_read_only_snapshot(snapshot: &UiSnapshot) -> UiSnapshot {
    let mut ro = snapshot.clone();
    if ro.phase != "deleted" {
        ro.phase = "read_only".into();
    }
    for node in ro.nodes.values_mut() {
        node.actions.clear();
        node.bindings.clear();
    }
    ro
}

fn trim_revisions(revisions: &mut BTreeMap<u64, UiSnapshot>) {
    while revisions.len() > MAX_STORED_REVISIONS {
        if let Some(oldest) = revisions.keys().next().copied() {
            revisions.remove(&oldest);
        } else {
            break;
        }
    }
}

fn extract_slot_ids(val: &Value) -> Vec<String> {
    match val {
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().filter(|s| !s.trim().is_empty()).map(ToString::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse LLM tool arguments (`components` array or `nodes` map) into normalized `UiNodeSnapshot`s.
pub fn build_nodes_from_spec(raw: &Value) -> Result<(Option<String>, BTreeMap<String, UiNodeSnapshot>), String> {
    let mut nodes = BTreeMap::new();
    let explicit_root = raw
        .get("root_instance_id")
        .or_else(|| raw.get("root_id"))
        .or_else(|| raw.get("root"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let mut first_id: Option<String> = None;

    if let Some(arr) = raw.get("components").and_then(|v| v.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let Some(obj) = item.as_object() else {
                return Err(format!("component[{idx}] must be an object"));
            };
            let id = obj
                .get("instance_id")
                .or_else(|| obj.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("node_{idx}"));
            if first_id.is_none() {
                first_id = Some(id.clone());
            }
            let component = obj
                .get("component")
                .or_else(|| obj.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("Text")
                .to_string();
            if !SUPPORTED_COMPONENTS.contains(&component.as_str()) {
                return Err(format!(
                    "unsupported A2UI component '{component}' (supported: {})",
                    SUPPORTED_COMPONENTS.join(", ")
                ));
            }
            let mut props = obj
                .get("props")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            // Copy ergonomic top-level fields into props if not already present
            for key in [
                "text",
                "label",
                "placeholder",
                "variant",
                "value",
                "options",
                "required",
                "minLength",
                "maxLength",
                "min",
                "max",
                "step",
                "steps",
                "axis",
                "displayStyle",
                "filterable",
                "name",
                "ariaLabel",
                "weight",
                "disabled",
                "tab_titles",
                "enableDate",
                "enableTime",
                "direction",
                "justify",
                "align",
            ] {
                if !props.contains_key(key) {
                    if let Some(val) = obj.get(key) {
                        props.insert(key.into(), val.clone());
                    }
                }
            }
            // Alias `step` -> `steps` for Slider parity with A2UI Basic Catalog
            if component == "Slider" && !props.contains_key("steps") {
                if let Some(step_val) = props.get("step").or_else(|| obj.get("step")).cloned() {
                    props.insert("steps".into(), step_val);
                }
            }
            // Default DateTimeInput `enableDate: true` when neither enableDate nor enableTime is set
            if component == "DateTimeInput"
                && !props.contains_key("enableDate")
                && !props.contains_key("enableTime")
            {
                props.insert("enableDate".into(), Value::Bool(true));
            }

            let mut slots = BTreeMap::new();
            if let Some(raw_slots) = obj.get("slots").and_then(|v| v.as_object()) {
                for (slot_name, ids_val) in raw_slots {
                    let ids = extract_slot_ids(ids_val);
                    if !ids.is_empty() || ids_val.is_array() {
                        slots.insert(slot_name.clone(), ids);
                    }
                }
            }
            if let Some(child_id) = obj.get("child").and_then(|v| v.as_str()) {
                slots
                    .entry("child".into())
                    .or_insert_with(|| vec![child_id.to_string()]);
            }
            if component == "Modal" {
                if let Some(trig) = obj.get("trigger") {
                    let ids = extract_slot_ids(trig);
                    if !ids.is_empty() {
                        slots.entry("trigger".into()).or_insert(ids);
                    }
                }
                if let Some(cont) = obj.get("content") {
                    let ids = extract_slot_ids(cont);
                    if !ids.is_empty() {
                        slots.entry("content".into()).or_insert(ids);
                    }
                }
            }
            if let Some(children) = obj.get("children").and_then(|v| v.as_array()) {
                if component == "Tabs" {
                    for (tab_idx, child_val) in children.iter().enumerate() {
                        let ids = extract_slot_ids(child_val);
                        if !ids.is_empty() {
                            slots.entry(format!("tab:{tab_idx}")).or_insert(ids);
                        }
                    }
                }
                slots.entry("children".into()).or_insert_with(|| {
                    children
                        .iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                });
            }

            let mut bindings = BTreeMap::new();
            if let Some(raw_bindings) = obj.get("bindings").and_then(|v| v.as_object()) {
                for (b_name, ptr) in raw_bindings {
                    if let Some(s) = ptr.as_str() {
                        bindings.insert(b_name.clone(), s.to_string());
                    }
                }
            }
            if let Some(bind_path) = obj.get("bind").and_then(|v| v.as_str()) {
                bindings.entry("value".into()).or_insert_with(|| bind_path.to_string());
            }

            let mut actions = BTreeMap::new();
            if let Some(raw_actions) = obj.get("actions").and_then(|v| v.as_object()) {
                for (a_name, a_val) in raw_actions {
                    if let Ok(a_ref) = serde_json::from_value::<UiActionRef>(a_val.clone()) {
                        actions.insert(a_name.clone(), a_ref);
                    } else if let Some(action_id) = a_val.as_str() {
                        actions.insert(
                            a_name.clone(),
                            UiActionRef {
                                instance_id: id.clone(),
                                action_id: action_id.to_string(),
                            },
                        );
                    }
                }
            }
            if let Some(action_id) = obj
                .get("action_id")
                .or_else(|| obj.get("action"))
                .and_then(|v| v.as_str())
            {
                actions.entry("action".into()).or_insert(UiActionRef {
                    instance_id: id.clone(),
                    action_id: action_id.to_string(),
                });
            }

            nodes.insert(
                id.clone(),
                UiNodeSnapshot {
                    instance_id: id.clone(),
                    component_id: obj
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string(),
                    component,
                    props,
                    slots,
                    bindings,
                    actions,
                    validation_errors: Vec::new(),
                },
            );
        }
    } else if let Some(map) = raw.get("nodes").and_then(|v| v.as_object()) {
        for (id, node_val) in map {
            let mut node: UiNodeSnapshot = serde_json::from_value(node_val.clone())
                .map_err(|e| format!("invalid node '{id}': {e}"))?;
            if !SUPPORTED_COMPONENTS.contains(&node.component.as_str()) {
                return Err(format!(
                    "unsupported A2UI component '{}' (supported: {})",
                    node.component,
                    SUPPORTED_COMPONENTS.join(", ")
                ));
            }
            if node.instance_id.is_empty() {
                node.instance_id = id.clone();
            }
            if first_id.is_none() {
                first_id = Some(node.instance_id.clone());
            }
            nodes.insert(node.instance_id.clone(), node);
        }
    } else {
        return Err("missing 'components' array or 'nodes' map".into());
    }

    let root = explicit_root.or(first_id);
    if let Some(ref r) = root {
        if !nodes.contains_key(r) {
            return Err(format!("root component '{r}' not found in components"));
        }
    }
    Ok((root, nodes))
}

/// Upsert a surface in SQLite and return the projected `UiSnapshot`.
pub fn upsert_ui_surface(
    store: &Store,
    owner: UiSurfaceOwner,
    surface_id: &str,
    summary: &str,
    root_instance_id: Option<String>,
    raw_nodes: BTreeMap<String, UiNodeSnapshot>,
    data_model: Value,
) -> Result<UiSnapshot, String> {
    if summary.len() > MAX_SUMMARY_BYTES {
        return Err(format!(
            "UI surface summary exceeds {} byte budget ({} bytes)",
            MAX_SUMMARY_BYTES,
            summary.len()
        ));
    }
    if raw_nodes.len() > MAX_NODES_PER_SURFACE {
        return Err(format!(
            "UI surface exceeds max {} nodes ({} provided)",
            MAX_NODES_PER_SURFACE,
            raw_nodes.len()
        ));
    }

    let key = surface_storage_key(&owner.conversation_id, surface_id);
    let existing: Option<StoredUiSurface> = store
        .get_module_record(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?;

    let needs_new_active_slot = match &existing {
        None => true,
        Some(prev) => prev.current.phase == "deleted",
    };
    if needs_new_active_slot {
        let active_count = list_conversation_surfaces(store, &owner.conversation_id)
            .into_iter()
            .filter(|s| s.phase != "deleted")
            .count();
        if active_count >= MAX_ACTIVE_SURFACES_PER_CONVERSATION {
            return Err(format!(
                "conversation '{}' already has the maximum {} active UI surfaces",
                owner.conversation_id, MAX_ACTIVE_SURFACES_PER_CONVERSATION
            ));
        }
    }

    let (generation, revision, mut revisions) = match existing {
        Some(prev) if prev.current.phase == "deleted" => (
            prev.current.identity.generation.saturating_add(1),
            1,
            BTreeMap::new(),
        ),
        Some(prev) => (
            prev.current.identity.generation,
            prev.current.revision + 1,
            prev.revisions,
        ),
        None => (1, 1, BTreeMap::new()),
    };

    let projected = project_nodes(&raw_nodes, &data_model);
    let identity = UiSurfaceIdentity {
        owner: owner.clone(),
        surface_id: surface_id.to_string(),
        generation,
    };
    let snapshot = UiSnapshot {
        identity,
        revision,
        catalog_id: DEFAULT_CATALOG_ID.into(),
        catalog_digest: catalog_digest(),
        phase: "ready".into(),
        root_instance_id,
        nodes: projected,
        summary: summary.to_string(),
    };

    let payload_size = serde_json::to_vec(&json!({
        "snapshot": &snapshot,
        "data_model": &data_model
    }))
    .map(|v| v.len())
    .unwrap_or(0);
    if payload_size > MAX_SURFACE_PAYLOAD_BYTES {
        return Err(format!(
            "UI surface serialized payload exceeds {} byte budget ({} bytes)",
            MAX_SURFACE_PAYLOAD_BYTES, payload_size
        ));
    }

    revisions.insert(revision, to_read_only_snapshot(&snapshot));
    trim_revisions(&mut revisions);

    let stored = StoredUiSurface {
        storage_key: key.clone(),
        conversation_id: owner.conversation_id,
        surface_id: surface_id.to_string(),
        current: snapshot.clone(),
        data_model,
        revisions,
        updated_at_ms: now_ms(),
    };
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;
    Ok(snapshot)
}

pub fn list_conversation_surfaces(store: &Store, conversation_id: &str) -> Vec<UiSnapshot> {
    store
        .list_module_records::<StoredUiSurface>(KIND_UI_SURFACE)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.conversation_id == conversation_id)
        .map(|s| s.current)
        .collect()
}

pub fn read_ui_surface_revision(
    store: &Store,
    reference: &UiSurfaceRevisionRef,
) -> Result<Option<UiSnapshot>, String> {
    let key = surface_storage_key(
        &reference.identity.owner.conversation_id,
        &reference.identity.surface_id,
    );
    let Some(stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    if let Some(rev) = stored.revisions.get(&reference.revision) {
        return Ok(Some(rev.clone()));
    }
    if stored.current.revision == reference.revision {
        return Ok(Some(to_read_only_snapshot(&stored.current)));
    }
    Ok(None)
}

pub fn read_ui_surface_changes(
    store: &Store,
    identity: &UiSurfaceIdentity,
    base_revision: Option<u64>,
) -> Result<Value, String> {
    let key = surface_storage_key(&identity.owner.conversation_id, &identity.surface_id);
    let Some(stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(json!({ "type": "missing", "identity": identity }));
    };
    if base_revision == Some(stored.current.revision) {
        return Ok(json!({
            "type": "unchanged",
            "identity": stored.current.identity,
            "revision": stored.current.revision
        }));
    }
    Ok(json!({
        "type": "snapshot",
        "snapshot": stored.current
    }))
}

pub fn preview_ui_surface(
    store: &Store,
    input: &UiDraftPreviewInput,
) -> Result<UiDraftPreview, String> {
    let key = surface_storage_key(
        &input.identity.owner.conversation_id,
        &input.identity.surface_id,
    );
    let stored = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "UI surface not found".to_string())?;
    if stored.current.revision != input.expected_surface_revision {
        return Err(format!(
            "revision conflict: expected {}, actual {}",
            input.expected_surface_revision, stored.current.revision
        ));
    }
    let mut draft_model = stored.data_model.clone();
    apply_edits_to_model(&stored.current.nodes, &mut draft_model, &input.edits)?;
    let projected_nodes = project_nodes(&stored.current.nodes, &draft_model);
    let mut snapshot = stored.current.clone();
    snapshot.nodes = projected_nodes;
    Ok(UiDraftPreview {
        identity: stored.current.identity,
        base_revision: stored.current.revision,
        draft_generation: input.draft_generation,
        snapshot,
    })
}

pub fn commit_ui_interaction(
    store: &Store,
    interaction: &UiInteraction,
) -> Result<CommittedInteraction, String> {
    let interaction_bytes = serde_json::to_vec(interaction)
        .map(|v| v.len())
        .unwrap_or(0);
    if interaction_bytes > MAX_INTERACTION_PAYLOAD_BYTES {
        return Err(format!(
            "UI interaction payload exceeds {} byte budget ({} bytes)",
            MAX_INTERACTION_PAYLOAD_BYTES, interaction_bytes
        ));
    }

    let key = surface_storage_key(
        &interaction.identity.owner.conversation_id,
        &interaction.identity.surface_id,
    );
    let mut stored = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "UI surface not found".to_string())?;
    if stored.current.phase == "deleted" {
        return Err("cannot interact with a deleted UI surface".into());
    }
    if stored.current.identity.generation != interaction.identity.generation {
        return Err(format!(
            "generation conflict: expected {}, actual {}",
            interaction.identity.generation, stored.current.identity.generation
        ));
    }
    if stored.current.revision != interaction.expected_surface_revision {
        return Err(format!(
            "revision conflict: expected {}, actual {}",
            interaction.expected_surface_revision, stored.current.revision
        ));
    }
    let mut next_model = stored.data_model.clone();
    apply_edits_to_model(&stored.current.nodes, &mut next_model, &interaction.edits)?;
    let projected_nodes = project_nodes(&stored.current.nodes, &next_model);

    let all_errors: Vec<String> = projected_nodes
        .values()
        .flat_map(|n| n.validation_errors.iter().cloned())
        .collect();
    if !all_errors.is_empty() && interaction.action.is_some() {
        return Err(format!("validation failed: {}", all_errors.join("; ")));
    }

    let action_id = interaction.action.as_ref().map(|a| a.action_id.clone());
    if let Some(ref act) = interaction.action {
        let valid_action = projected_nodes
            .get(&act.instance_id)
            .is_some_and(|n| n.actions.values().any(|a| a.action_id == act.action_id));
        if !valid_action {
            return Err(format!(
                "action '{}' not found on node '{}'",
                act.action_id, act.instance_id
            ));
        }
    }

    let next_rev = stored.current.revision + 1;
    stored.current.revision = next_rev;
    stored.current.nodes = projected_nodes;
    stored.data_model = next_model.clone();
    stored
        .revisions
        .insert(next_rev, to_read_only_snapshot(&stored.current));
    trim_revisions(&mut stored.revisions);
    stored.updated_at_ms = now_ms();
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;

    let summary = match &action_id {
        Some(aid) => format!(
            "Submitted UI action '{}' on surface '{}' with data: {}",
            aid, stored.surface_id, next_model
        ),
        None => format!(
            "Updated UI surface '{}' with data: {}",
            stored.surface_id, next_model
        ),
    };
    Ok(CommittedInteraction {
        snapshot: stored.current,
        data_model: next_model,
        action_id,
        summary,
    })
}

pub fn delete_ui_surface(
    store: &Store,
    conversation_id: &str,
    surface_id: &str,
) -> Result<Option<UiSnapshot>, String> {
    let key = surface_storage_key(conversation_id, surface_id);
    let Some(mut stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    stored.current.revision += 1;
    stored.current.phase = "deleted".into();
    stored.current.root_instance_id = None;
    stored.current.nodes.clear();
    stored
        .revisions
        .insert(stored.current.revision, stored.current.clone());
    trim_revisions(&mut stored.revisions);
    stored.updated_at_ms = now_ms();
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;
    Ok(Some(stored.current))
}

/// Register A2UI tools on a `ToolRegistry`.
pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    register_scoped_tools(suite, registry, "local-host", "default");
}

/// Register A2UI tools bound to a specific conversation + host identity (used by WebHost / Desktop).
pub fn register_scoped_tools(
    suite: &Arc<ModulesSuite>,
    registry: &mut ToolRegistry,
    authority_id: &str,
    conversation_id: &str,
) {
    registry.register(Arc::new(RenderUiSurfaceTool {
        suite: Arc::clone(suite),
        authority_id: authority_id.to_string(),
        conversation_id: conversation_id.to_string(),
    }));
    registry.register(Arc::new(DeleteUiSurfaceTool {
        suite: Arc::clone(suite),
        conversation_id: conversation_id.to_string(),
    }));
}

struct RenderUiSurfaceTool {
    suite: Arc<ModulesSuite>,
    authority_id: String,
    conversation_id: String,
}

impl ToolHandler for RenderUiSurfaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "render_ui_surface".into(),
                description: "Render or update an interactive A2UI Generative UI surface (forms, cards, choices, sliders, buttons) inside the conversation.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "surface_id": { "type": "string", "description": "Stable identifier for this UI surface (e.g. 'deploy_form')" },
                        "summary": { "type": "string", "description": "Short human-readable summary of the surface" },
                        "root_id": { "type": "string", "description": "ID of the root component" },
                        "components": {
                            "type": "array",
                            "description": "Flat list of A2UI Basic Catalog components (Card, Column, Row, Text, Button, TextField, CheckBox, ChoicePicker, Slider, DateTimeInput, Tabs, Modal, List, Divider, Icon)",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "component": { "type": "string" },
                                    "props": { "type": "object" },
                                    "children": { "type": "array", "items": { "type": "string" } },
                                    "child": { "type": "string" },
                                    "bind": { "type": "string" },
                                    "action_id": { "type": "string" }
                                },
                                "required": ["id", "component"]
                            }
                        },
                        "data_model": { "type": "object", "description": "Initial JSON data model for two-way component bindings" }
                    },
                    "required": ["surface_id", "components"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(surface_id) = args
            .get("surface_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            return ToolExecutionResult::err_structured(
                "missing_param",
                "missing non-empty 'surface_id'",
                None,
            );
        };
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("Interactive UI Surface");
        let data_model = args.get("data_model").cloned().unwrap_or_else(|| json!({}));
        let (root_id, nodes) = match build_nodes_from_spec(&args) {
            Ok(pair) => pair,
            Err(e) => return ToolExecutionResult::err_structured("invalid_ui_spec", e, None),
        };
        let owner = UiSurfaceOwner {
            authority_id: self.authority_id.clone(),
            conversation_id: self.conversation_id.clone(),
            participant_id: "assistant".into(),
            context_id: "conversation".into(),
            originating_run_id: new_id(),
            message_id: format!("ui-{}", now_ms()),
            part_id: format!("part-{surface_id}"),
        };
        match upsert_ui_surface(
            &self.suite.store,
            owner,
            surface_id,
            summary,
            root_id,
            nodes,
            data_model,
        ) {
            Ok(snapshot) => ToolExecutionResult::ok_structured(
                json!({
                    "ui_surface": snapshot,
                    "surface_id": surface_id,
                    "revision": snapshot.revision
                }),
                format!(
                    "Rendered A2UI surface '{}' (rev {}, {} nodes)",
                    surface_id,
                    snapshot.revision,
                    snapshot.nodes.len()
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
        }
    }
}

struct DeleteUiSurfaceTool {
    suite: Arc<ModulesSuite>,
    conversation_id: String,
}

impl ToolHandler for DeleteUiSurfaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "delete_ui_surface".into(),
                description: "Remove an existing interactive A2UI surface from the conversation.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "surface_id": { "type": "string" }
                    },
                    "required": ["surface_id"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(surface_id) = args.get("surface_id").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'surface_id'", None);
        };
        match delete_ui_surface(&self.suite.store, &self.conversation_id, surface_id) {
            Ok(Some(snapshot)) => ToolExecutionResult::ok_structured(
                json!({"ui_surface": snapshot, "deleted": true}),
                format!("Deleted A2UI surface '{surface_id}'"),
            ),
            Ok(None) => ToolExecutionResult::ok_structured(
                json!({"deleted": false, "surface_id": surface_id}),
                format!("UI surface '{surface_id}' did not exist"),
            ),
            Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a2ui_surface_lifecycle_preview_validation_interaction_and_history() {
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), std::path::PathBuf::from("."), None));
        let mut reg = ToolRegistry::with_builtin_tools();
        register_scoped_tools(&suite, &mut reg, "host-1", "conv-1");
        let ctx = ToolContext::new(".");

        let res = reg.execute(
            "render_ui_surface",
            &ctx,
            json!({
                "surface_id": "deploy_form",
                "summary": "Deploy Configuration",
                "root_id": "card",
                "components": [
                    { "id": "card", "component": "Card", "child": "col" },
                    { "id": "col", "component": "Column", "children": ["title", "env_input", "submit_btn"] },
                    { "id": "title", "component": "Text", "text": "Deploy Service", "variant": "h2" },
                    { "id": "env_input", "component": "TextField", "label": "Environment", "bind": "/env", "required": true, "minLength": 3 },
                    { "id": "submit_btn", "component": "Button", "variant": "primary", "child": "btn_label", "action_id": "confirm_deploy" },
                    { "id": "btn_label", "component": "Text", "text": "Deploy Now" }
                ],
                "data_model": { "env": "" }
            }),
        );
        assert!(!res.is_error, "render_ui_surface failed: {}", res.payload.to_model_value());
        let snap: UiSnapshot = serde_json::from_value(res.payload.to_model_value()["ui_surface"].clone()).unwrap();
        assert_eq!(snap.revision, 1);
        assert_eq!(snap.phase, "ready");
        assert!(!snap.nodes["env_input"].validation_errors.is_empty());

        // Preview invalid short value
        let bad_preview = preview_ui_surface(
            &store,
            &UiDraftPreviewInput {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("ab"),
                }],
                draft_generation: 2,
            },
        )
        .unwrap();
        assert!(!bad_preview.snapshot.nodes["env_input"].validation_errors.is_empty());

        // Preview valid value
        let ok_preview = preview_ui_surface(
            &store,
            &UiDraftPreviewInput {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("production"),
                }],
                draft_generation: 3,
            },
        )
        .unwrap();
        assert!(ok_preview.snapshot.nodes["env_input"].validation_errors.is_empty());
        assert_eq!(ok_preview.snapshot.nodes["env_input"].props["value"], json!("production"));

        // Commit interaction
        let committed = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("production"),
                }],
                action: Some(UiActionRef {
                    instance_id: "submit_btn".into(),
                    action_id: "confirm_deploy".into(),
                }),
            },
        )
        .unwrap();
        assert_eq!(committed.snapshot.revision, 2);
        assert_eq!(committed.action_id.as_deref(), Some("confirm_deploy"));
        assert_eq!(committed.data_model["env"], json!("production"));

        // Historical revision 1 is strictly read_only with stripped actions/bindings
        let hist = read_ui_surface_revision(
            &store,
            &UiSurfaceRevisionRef {
                identity: snap.identity.clone(),
                revision: 1,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(hist.phase, "read_only");
        assert!(hist.nodes.values().all(|n| n.actions.is_empty() && n.bindings.is_empty()));

        // Delete surface
        let deleted = delete_ui_surface(&store, "conv-1", "deploy_form").unwrap().unwrap();
        assert_eq!(deleted.phase, "deleted");
    }

    fn test_owner(conv_id: &str) -> UiSurfaceOwner {
        UiSurfaceOwner {
            authority_id: "host-1".into(),
            conversation_id: conv_id.into(),
            participant_id: "assistant".into(),
            context_id: "conversation".into(),
            originating_run_id: "run-1".into(),
            message_id: "msg-1".into(),
            part_id: "part-1".into(),
        }
    }

    #[test]
    fn a2ui_enforces_generation_bump_disabled_edits_and_budgets() {
        let store = Store::open_in_memory().unwrap();
        let (_, nodes) = build_nodes_from_spec(&json!({
            "root_id": "locked",
            "components": [
                { "id": "locked", "component": "TextField", "label": "Locked", "bind": "/val", "disabled": true }
            ]
        }))
        .unwrap();

        // 1. Create -> delete -> recreate bumps generation and resets revision to 1
        let s1 = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            "First generation",
            Some("locked".into()),
            nodes.clone(),
            json!({"val": "initial"}),
        )
        .unwrap();
        assert_eq!(s1.identity.generation, 1);
        assert_eq!(s1.revision, 1);

        // Disabled node rejects edits
        let disabled_err = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: s1.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "locked".into(),
                    binding: "value".into(),
                    value: json!("hacked"),
                }],
                action: None,
            },
        )
        .unwrap_err();
        assert!(disabled_err.contains("disabled"));

        // Delete & recreate
        let del = delete_ui_surface(&store, "conv-g", "panel").unwrap().unwrap();
        assert_eq!(del.phase, "deleted");

        let s2 = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            "Second generation",
            Some("locked".into()),
            nodes.clone(),
            json!({"val": "fresh"}),
        )
        .unwrap();
        assert_eq!(s2.identity.generation, 2);
        assert_eq!(s2.revision, 1);
        assert_eq!(s2.phase, "ready");

        // 2. Summary budget enforced (> 4096 bytes)
        let huge_summary = "x".repeat(MAX_SUMMARY_BYTES + 1);
        let summary_err = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            &huge_summary,
            Some("locked".into()),
            nodes.clone(),
            json!({}),
        )
        .unwrap_err();
        assert!(summary_err.contains("summary"));

        // 3. Max 8 active surfaces per conversation enforced
        for i in 0..MAX_ACTIVE_SURFACES_PER_CONVERSATION {
            upsert_ui_surface(
                &store,
                test_owner("conv-limit"),
                &format!("surf-{i}"),
                "ok",
                Some("locked".into()),
                nodes.clone(),
                json!({}),
            )
            .unwrap();
        }
        let limit_err = upsert_ui_surface(
            &store,
            test_owner("conv-limit"),
            "surf-overflow",
            "ok",
            Some("locked".into()),
            nodes.clone(),
            json!({}),
        )
        .unwrap_err();
        assert!(limit_err.contains("maximum 8"));

        // 4. Interaction payload budget (> 64 KiB)
        let huge_interaction_err = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: s2.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "locked".into(),
                    binding: "value".into(),
                    value: json!("a".repeat(MAX_INTERACTION_PAYLOAD_BYTES + 100)),
                }],
                action: None,
            },
        )
        .unwrap_err();
        assert!(huge_interaction_err.contains("budget"));
    }

    #[test]
    fn a2ui_basic_catalog_parity_and_revision_cap() {
        let store = Store::open_in_memory().unwrap();
        let (_, nodes) = build_nodes_from_spec(&json!({
            "root_id": "tabs",
            "components": [
                { "id": "tabs", "component": "Tabs", "tab_titles": ["A", "B"], "children": ["modal", "dt"] },
                { "id": "modal", "component": "Modal", "trigger": "open_btn", "content": "slider" },
                { "id": "open_btn", "component": "Button", "text": "Open" },
                { "id": "slider", "component": "Slider", "min": 0, "max": 100, "step": 5 },
                { "id": "dt", "component": "DateTimeInput", "label": "When" },
                { "id": "div", "component": "Divider", "axis": "vertical" },
                { "id": "cp", "component": "ChoicePicker", "displayStyle": "chips", "filterable": true }
            ]
        }))
        .unwrap();
        assert_eq!(nodes["tabs"].slots["tab:0"], vec!["modal"]);
        assert_eq!(nodes["tabs"].slots["tab:1"], vec!["dt"]);
        assert_eq!(nodes["modal"].slots["trigger"], vec!["open_btn"]);
        assert_eq!(nodes["modal"].slots["content"], vec!["slider"]);
        assert_eq!(nodes["slider"].props["steps"], json!(5));
        assert_eq!(nodes["dt"].props["enableDate"], json!(true));
        assert_eq!(nodes["div"].props["axis"], json!("vertical"));
        assert_eq!(nodes["cp"].props["displayStyle"], json!("chips"));
        assert_eq!(nodes["cp"].props["filterable"], json!(true));

        // Invalid component rejected in nodes map branch too
        let bad_nodes = build_nodes_from_spec(&json!({
            "nodes": {
                "x": {
                    "instance_id": "x",
                    "component_id": "x",
                    "component": "EvilScriptTag",
                    "props": {},
                    "slots": {},
                    "bindings": {},
                    "actions": {},
                    "validation_errors": []
                }
            }
        }));
        assert!(bad_nodes.is_err());

        // Revisions capped to 20
        for _ in 0..25 {
            upsert_ui_surface(
                &store,
                test_owner("conv-rev"),
                "surf",
                "summary",
                Some("tabs".into()),
                nodes.clone(),
                json!({}),
            )
            .unwrap();
        }
        let key = surface_storage_key("conv-rev", "surf");
        let stored: StoredUiSurface = store
            .get_module_record(KIND_UI_SURFACE, &key)
            .unwrap()
            .unwrap();
        assert_eq!(stored.revisions.len(), 20);
        assert_eq!(*stored.revisions.keys().next().unwrap(), 6);
        assert_eq!(*stored.revisions.keys().last().unwrap(), 25);
    }
}
