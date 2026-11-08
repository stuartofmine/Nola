// SPDX-License-Identifier: Apache-2.0
//! Expand component graphs into resolved, platform-neutral view instances.
use super::{data::{absolute_pointer, overlapping_paths, read_pointer}, resolve::Evaluator};
use nola_domain::generative_ui::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::{cell::Cell, rc::Rc};

pub struct Projection {
    pub snapshot: UiSnapshot,
    pub dependencies: BTreeMap<String, BTreeSet<String>>,
    pub scopes: BTreeMap<String, String>,
}

pub fn project_surface(state: &UiSurfaceState) -> Result<Projection, UiError> {
    if state.catalog_id != super::schema::NOLA_CATALOG_ID
        || state.catalog_digest != super::schema::catalog_digest() {
        return Err(UiError::UnsupportedCatalog);
    }
    let mut builder = Builder { state, protected_identity_paths: BTreeSet::new(), editable_components: BTreeMap::new(), remaining_bytes: UI_MAX_SURFACE_BYTES, reserved_nodes: 0, summary: Vec::new(), summary_truncated: false, budget: Rc::new(Cell::new(10000)), nodes: BTreeMap::new(), dependencies: BTreeMap::new(), scopes: BTreeMap::new() };
    let root = if matches!(state.phase, UiSurfacePhase::Ready | UiSurfacePhase::ReadOnly) {
        Some(builder.node("root", "", "", &mut BTreeSet::new(), 0)?)
    } else { None };
    for node in builder.nodes.values() {
        if node.bindings.values().any(|binding| builder.protected_identity_paths.iter()
            .any(|identity| overlapping_paths(binding, identity))) {
            return Err(UiError::ImmutableListIdentity);
        }
    }
    let mut summary = builder.summary.join("\n");
    // A semantic transcript summary is bounded independently of rendered Text.
    // Preserve UTF-8 boundaries; component props retain their complete content.
    if builder.summary_truncated || summary.len() > UI_MAX_SUMMARY_BYTES {
        let mut end = summary.len().min(UI_MAX_SUMMARY_BYTES - "…".len());
        while !summary.is_char_boundary(end) { end -= 1; }
        summary.truncate(end);
        summary.push('…');
    }
    let snapshot = UiSnapshot { identity: state.identity.clone(), revision: state.revision,
        catalog_id: state.catalog_id.clone(), catalog_digest: state.catalog_digest.clone(),
        phase: state.phase, root_instance_id: root, nodes: builder.nodes, summary };
    // Incremental accounting bounds expansion work. The actual bridge payload
    // also includes object keys, identities and JSON-escaped summary strings.
    if serde_json::to_vec(&snapshot).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_SURFACE_BYTES {
        return Err(UiError::ResourceLimit);
    }
    Ok(Projection { snapshot, dependencies: builder.dependencies, scopes: builder.scopes })
}

/// Historical reads preserve identity/revision without mutating authoritative state.
pub fn project_historical_surface(state: &UiSurfaceState) -> Result<UiSnapshot, UiError> {
    let mut historical = state.clone();
    if historical.phase == UiSurfacePhase::Ready { historical.phase = UiSurfacePhase::ReadOnly; }
    project_surface(&historical).map(|projection| projection.snapshot)
}

struct Builder<'a> {
    state: &'a UiSurfaceState,
    protected_identity_paths: BTreeSet<String>,
    editable_components: BTreeMap<String, bool>,
    remaining_bytes: usize,
    reserved_nodes: usize,
    summary: Vec<String>,
    summary_truncated: bool,
    budget: Rc<Cell<usize>>,
    nodes: BTreeMap<String, UiNodeSnapshot>,
    dependencies: BTreeMap<String, BTreeSet<String>>,
    scopes: BTreeMap<String, String>,
}

impl Builder<'_> {
    fn node(&mut self, id: &str, scope: &str, instance_scope: &str, stack: &mut BTreeSet<String>, depth: usize) -> Result<String, UiError> {
        if depth > UI_MAX_DEPTH { return Err(UiError::ResourceLimit); }
        let instance_id = identity(id, instance_scope);
        if self.nodes.contains_key(&instance_id) { return Ok(instance_id); }
        // Reserve before descending: ancestors are not in nodes yet, but count
        // toward the same expanded-instance budget as their descendants.
        if self.reserved_nodes >= UI_MAX_NODES { return Err(UiError::ResourceLimit); }
        self.reserved_nodes += 1;
        if !stack.insert(id.to_owned()) { return Err(UiError::InvalidMessage); }
        let spec = self.state.components.get(id).ok_or(UiError::InvalidMessage)?.clone();
        let object = spec.as_object().ok_or(UiError::InvalidMessage)?;
        let component = object.get("component").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?;
        let mut evaluator = Evaluator::with_budget(&self.state.data, Rc::clone(&self.budget));
        let mut node = UiNodeSnapshot { instance_id: instance_id.clone(), component_id: id.to_owned(),
            component: component.to_owned(), props: BTreeMap::new(), slots: BTreeMap::new(),
            bindings: BTreeMap::new(), actions: BTreeMap::new(), validation_errors: Vec::new() };
        for (property, value) in object {
            match property.as_str() {
                "id" | "component" => {}
                "children" => {
                    let mut children = Vec::new();
                    if let Some(ids) = value.as_array() {
                        let mut occurrences = BTreeMap::<String, usize>::new();
                        for child in ids {
                            let child = child.as_str().ok_or(UiError::InvalidMessage)?;
                            let occurrence = occurrences.entry(child.to_owned()).or_default();
                            let placement = placement_scope(&instance_id, "children", child, *occurrence);
                            *occurrence += 1;
                            children.push(self.node(child, scope, &placement, stack, depth + 1)?);
                        }
                    } else {
                        let template = value.get("componentId").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?;
                        let path = absolute_pointer(scope, value.get("path").and_then(Value::as_str).ok_or(UiError::InvalidBinding)?)?;
                        evaluator.dependencies.insert(path.clone());
                        let collection = read_pointer(&self.state.data, &path)?.cloned().unwrap_or(Value::Array(Vec::new()));
                        let values = collection.as_array().ok_or(UiError::InvalidBinding)?;
                        if values.len() > UI_MAX_NODES { return Err(UiError::ResourceLimit); }
                        let editable = self.state.phase != UiSurfacePhase::ReadOnly
                            && has_editable_descendant(template, &self.state.components,
                                &mut self.editable_components, &mut BTreeSet::new())?;
                        let mut stable_keys = BTreeSet::new();
                        let mut occurrences = BTreeMap::<String, usize>::new();
                        for (index, item) in values.iter().enumerate() {
                            // Explicit data identity survives edits and reordering;
                            // value identity survives reordering of unkeyed items.
                            let stable_id = item.get("id").filter(|id| id.as_str()
                                .is_some_and(|text| !text.trim().is_empty()) || id.is_number());
                            if editable {
                                let stable_id = stable_id.ok_or(UiError::InvalidListIdentity)?;
                                let encoded = serde_json::to_string(stable_id).map_err(|_| UiError::InvalidMessage)?;
                                if !stable_keys.insert(encoded) { return Err(UiError::InvalidListIdentity); }
                                self.protected_identity_paths.insert(absolute_pointer(&path, &format!("{index}/id"))?);
                            }
                            let key_value = stable_id.unwrap_or(item);
                            let key = format!("{:x}", Sha256::digest(serde_json::to_vec(key_value).map_err(|_| UiError::InvalidMessage)?));
                            let occurrence = occurrences.entry(key.clone()).or_default();
                            let item_instance = placement_scope(&instance_id, "template", &key, *occurrence);
                            *occurrence += 1;
                            let item_scope = absolute_pointer(&path, &index.to_string())?;
                            children.push(self.node(template, &item_scope, &item_instance, stack, depth + 1)?);
                        }
                    }
                    node.slots.insert(property.clone(), children);
                }
                "child" | "trigger" | "content" => {
                    let child = value.as_str().ok_or(UiError::InvalidMessage)?;
                    let placement = placement_scope(&instance_id, property, child, 0);
                    node.slots.insert(property.clone(), vec![self.node(child, scope, &placement, stack, depth + 1)?]);
                }
                "tabs" => {
                    let mut labels = Vec::new();
                    for (index, tab) in value.as_array().ok_or(UiError::InvalidMessage)?.iter().enumerate() {
                        labels.push(evaluator.resolve(tab.get("title").ok_or(UiError::InvalidMessage)?, scope)?);
                        let child = tab.get("child").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?;
                        let slot = format!("tab:{index}");
                        let placement = placement_scope(&instance_id, &slot, child, 0);
                        node.slots.insert(slot, vec![self.node(child, scope, &placement, stack, depth + 1)?]);
                    }
                    node.props.insert("tab_titles".into(), Value::Array(labels));
                }
                "action" => {
                    node.actions.insert(property.clone(), UiActionRef { instance_id: instance_id.clone(), action_id: property.clone() });
                }
                "checks" => {
                    for check in value.as_array().ok_or(UiError::InvalidMessage)? {
                        let valid = evaluator.resolve(check.get("condition").ok_or(UiError::InvalidMessage)?, scope)?;
                        if valid.as_bool() != Some(true) {
                            node.validation_errors.push(check.get("message").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?.to_owned());
                        }
                    }
                }
                _ => {
                    if property == "value" {
                        if let Some(path) = value.as_object().filter(|o| o.len() == 1).and_then(|o| o.get("path")).and_then(Value::as_str) {
                            node.bindings.insert(property.clone(), absolute_pointer(scope, path)?);
                        }
                    }
                    node.props.insert(property.clone(), evaluator.resolve(value, scope)?);
                }
            }
        }
        if component == "TextField" {
            if let Some(pattern) = node.props.get("validationRegexp").and_then(Value::as_str) {
                let regex = super::resolve::bounded_regex(pattern)?;
                if let Some(value) = node.props.get("value").and_then(Value::as_str) {
                    if !regex.is_match(value) {
                        node.validation_errors.push("Value does not match the required format.".into());
                    }
                }
            }
        }
        if self.state.phase == UiSurfacePhase::ReadOnly {
            node.props.insert("disabled".into(), Value::Bool(true));
            node.actions.clear();
            node.bindings.clear();
        }
        // Bound expanded output as well as the compact source graph. A shared
        // binding can otherwise multiply a large string across many instances.
        let node_bytes = serde_json::to_vec(&node).map_err(|_| UiError::InvalidMessage)?.len();
        self.remaining_bytes = self.remaining_bytes.checked_sub(node_bytes).ok_or(UiError::ResourceLimit)?;
        // Text-only clients need form labels too. Do not copy editable values
        // into the tool summary: user data travels through explicit actions.
        let summary_property = match component {
            "Text" => Some("text"),
            "TextField" | "CheckBox" | "ChoicePicker" | "Slider" | "DateTimeInput" => Some("label"),
            _ => None,
        };
        if let Some(text) = summary_property.and_then(|key| node.props.get(key)).and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty()) {
            let used = self.summary.iter().map(String::len).sum::<usize>() + self.summary.len();
            let available = (UI_MAX_SUMMARY_BYTES + 1).saturating_sub(used);
            if available > 0 {
                let mut end = text.len().min(available);
                while !text.is_char_boundary(end) { end -= 1; }
                self.remaining_bytes = self.remaining_bytes.checked_sub(end).ok_or(UiError::ResourceLimit)?;
                self.summary.push(text[..end].to_owned());
                self.summary_truncated |= end < text.len();
            } else {
                self.summary_truncated = true;
            }
        }
        self.dependencies.insert(instance_id.clone(), evaluator.dependencies);
        self.scopes.insert(instance_id.clone(), scope.to_owned());
        self.nodes.insert(instance_id.clone(), node);
        stack.remove(id);
        Ok(instance_id)
    }
}

// The stable-key requirement follows the template graph, including nested
// templates. Read-only text/list rendering remains valid without identity keys.
fn has_editable_descendant(
    id: &str, components: &BTreeMap<String, Value>, cache: &mut BTreeMap<String, bool>,
    visiting: &mut BTreeSet<String>,
) -> Result<bool, UiError> {
    if let Some(editable) = cache.get(id) { return Ok(*editable); }
    if visiting.len() > UI_MAX_DEPTH { return Err(UiError::ResourceLimit); }
    if !visiting.insert(id.to_owned()) { return Err(UiError::InvalidMessage); }
    let component = components.get(id).ok_or(UiError::InvalidMessage)?;
    let mut editable = matches!(component.get("component").and_then(Value::as_str),
        Some("TextField" | "DateTimeInput" | "CheckBox" | "Slider" | "ChoicePicker"))
        && component.get("value").and_then(Value::as_object)
            .is_some_and(|value| value.len() == 1 && value.get("path").is_some_and(Value::is_string));
    for child in super::state::child_ids(component)? {
        editable |= has_editable_descendant(&child, components, cache, visiting)?;
    }
    visiting.remove(id);
    cache.insert(id.to_owned(), editable);
    Ok(editable)
}

// Placement and binding scope are distinct: two views may share a data path,
// but must not share native focus, tab selection or disclosure state.
fn placement_scope(parent: &str, slot: &str, child: &str, occurrence: usize) -> String {
    let mut digest = Sha256::new();
    for part in [parent, slot, child] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    digest.update((occurrence as u64).to_be_bytes());
    format!("placement_{:x}", digest.finalize())
}

fn identity(component_id: &str, scope: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((component_id.len() as u64).to_be_bytes());
    digest.update(component_id.as_bytes());
    digest.update(scope.as_bytes());
    format!("node_{:x}", digest.finalize())
}

pub fn changes(previous: &UiSnapshot, next: &UiSnapshot) -> Result<UiChangeSet, UiError> {
    if previous.identity != next.identity || previous.catalog_digest != next.catalog_digest { return Err(UiError::StaleOwner); }
    if next.revision <= previous.revision { return Err(UiError::RevisionConflict); }
    Ok(UiChangeSet { identity: next.identity.clone(), base_revision: previous.revision, revision: next.revision,
        catalog_digest: next.catalog_digest.clone(), phase: next.phase, root_instance_id: next.root_instance_id.clone(),
        upserted_nodes: next.nodes.iter().filter(|(id, node)| previous.nodes.get(*id) != Some(*node)).map(|(_, node)| node.clone()).collect(),
        removed_instance_ids: previous.nodes.keys().filter(|id| !next.nodes.contains_key(*id)).cloned().collect(),
        summary: next.summary.clone() })
}


#[cfg(test)]
mod transport_budget_tests {
    use super::*;
    use serde_json::json;

    fn text_surface(text: &str) -> UiSurfaceState {
        serde_json::from_value(json!({
            "identity": {"owner": {"authority_id":"host", "conversation_id":"chat",
                "participant_id":"assistant", "context_id":"chat", "originating_run_id":"run",
                "message_id":"message", "part_id":"part"}, "surface_id":"text", "generation":1},
            "revision":1, "catalog_id":super::super::schema::NOLA_CATALOG_ID,
            "catalog_digest":super::super::schema::catalog_digest(), "phase":"ready",
            "components":{"root":{"id":"root","component":"Text","text":text}},
            "data":{}, "theme":null, "send_data_model":false
        })).unwrap()
    }

    #[test]
    fn historical_projection_retains_content_and_identity_without_edit_handles() {
        let mut state = text_surface("");
        state.components.insert("root".into(), json!({"id":"root","component":"TextField",
            "label":"Name","value":{"path":"/name"}}));
        state.data = json!({"name":"saved value"});
        let before = serde_json::to_value(&state).unwrap();
        let live = project_surface(&state).unwrap().snapshot;
        assert!(!live.nodes.values().next().unwrap().bindings.is_empty());
        let historical = project_historical_surface(&state).unwrap();
        assert_eq!(historical.identity, state.identity);
        assert_eq!(historical.revision, state.revision);
        assert_eq!(historical.phase, UiSurfacePhase::ReadOnly);
        for node in historical.nodes.values() {
            assert!(node.bindings.is_empty() && node.actions.is_empty());
            assert_eq!(node.props["disabled"], true);
            assert_eq!(node.props["value"], "saved value");
        }
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[test]
    fn form_only_summary_contains_labels_without_editable_values() {
        let mut state = text_surface("");
        state.components.insert("root".into(), json!({"id":"root","component":"Column","children":["name","consent"]}));
        state.components.insert("name".into(), json!({"id":"name","component":"TextField",
            "label":"姓名", "value":{"path":"/name"}}));
        state.components.insert("consent".into(), json!({"id":"consent","component":"CheckBox",
            "label":"确认提交", "value":false}));
        state.data = json!({"name":"private field value"});
        let projection = project_surface(&state).unwrap();
        assert_eq!(projection.snapshot.summary, "姓名\n确认提交");
        assert!(!projection.snapshot.summary.contains("private field value"));
    }

    #[test]
    fn escaped_summary_counts_toward_complete_bridge_budget() {
        let state = text_surface(&"\\".repeat(UI_MAX_SURFACE_BYTES / 2 - UI_MAX_SUMMARY_BYTES / 2));
        assert!(serde_json::to_vec(&state).unwrap().len() < UI_MAX_SURFACE_BYTES);
        assert!(matches!(project_surface(&state), Err(UiError::ResourceLimit)));
        let accepted = project_surface(&text_surface(&"\\".repeat(100_000))).unwrap();
        assert!(serde_json::to_vec(&accepted.snapshot).unwrap().len() <= UI_MAX_SURFACE_BYTES);
    }
}

#[cfg(test)]
mod stable_list_identity_tests {
    use super::*;
    use crate::generative_ui::interaction::prepare_interaction;
    use serde_json::json;

    fn list(editable: bool, items: Value) -> UiSurfaceState {
        let mut state: UiSurfaceState = serde_json::from_value(json!({
            "identity": {"owner": {"authority_id":"host", "conversation_id":"chat",
                "participant_id":"assistant", "context_id":"chat", "originating_run_id":"run",
                "message_id":"message", "part_id":"part"}, "surface_id":"list", "generation":1},
            "revision":1, "catalog_id":super::super::schema::NOLA_CATALOG_ID,
            "catalog_digest":super::super::schema::catalog_digest(), "phase":"ready",
            "components":{
                "root":{"id":"root","component":"Column","children":{"componentId":"row","path":"/items"}},
                "row":{"id":"row","component":"Column","children":["field","button"]},
                "field":{"id":"field","component":"TextField","label":"Name","value":{"path":"name"}},
                "button":{"id":"button","component":"Button","child":"label",
                    "action":{"event":{"name":"save","context":{"name":{"path":"name"}}}}},
                "label":{"id":"label","component":"Text","text":"Save"}
            }, "data":{"items":items}, "theme":null,"send_data_model":false
        })).unwrap();
        if !editable {
            state.components.insert("row".into(), json!({"id":"row","component":"Text","text":{"path":"name"}}));
        }
        state
    }

    #[test]
    fn template_under_empty_object_key_preserves_binding_and_action_scope() {
        let mut state = list(true, json!([]));
        state.data = json!({"items":{"": [{"id":"a","name":"Original"}]}});
        state.components.get_mut("root").unwrap()["children"]["path"] = json!("/items/");
        let projection = project_surface(&state).unwrap();
        let field = projection.snapshot.nodes.values().find(|node| node.component_id == "field").unwrap();
        assert_eq!(field.bindings["value"], "/items//0/name");
        assert_eq!(field.props["value"], "Original");
        let action = projection.snapshot.nodes.values().find(|node| node.component_id == "button").unwrap().actions["action"].clone();
        let result = prepare_interaction(&state, &UiInteraction {
            identity:state.identity.clone(), expected_surface_revision:1,
            edits:vec![UiEdit::Set { instance_id:field.instance_id.clone(), binding:"value".into(), value:json!("Edited") }],
            action:Some(action),
        }).unwrap();
        assert_eq!(result.state.data["items"][""][0]["name"], "Edited");
        assert_eq!(result.event.unwrap()["context"]["name"], "Edited");
        state.components.get_mut("field").unwrap()["value"] = json!({"path":"id"});
        assert!(matches!(project_surface(&state), Err(UiError::ImmutableListIdentity)));
    }

    #[test]
    fn editable_items_reject_missing_empty_invalid_and_duplicate_keys() {
        for items in [
            json!([{"name":"A"}]), json!([{"id":"","name":"A"}]),
            json!([{"id":"  ","name":"A"}]), json!([{"id":true,"name":"A"}]),
            json!([{"id":"a","name":"A"},{"id":"a","name":"B"}]),
            json!([{"id":7,"name":"A"},{"id":7,"name":"B"}]),
        ] {
            assert!(matches!(project_surface(&list(true, items)), Err(UiError::InvalidListIdentity)));
        }
        assert!(UiError::InvalidListIdentity.to_string().contains("retry the unchanged surface revision"));
    }

    #[test]
    fn readonly_unkeyed_and_duplicate_items_remain_renderable() {
        for items in [json!([{"name":"A"},{"name":"A"}]),
                      json!([{"id":"a","name":"A"},{"id":"a","name":"B"}])] {
            let snapshot = project_surface(&list(false, items)).unwrap().snapshot;
            assert_eq!(snapshot.nodes.len(), 3);
            assert!(snapshot.nodes.values().all(|node| node.bindings.is_empty()));
        }
    }

    #[test]
    fn historical_unkeyed_fields_remain_readable_without_interaction_handles() {
        let state = list(true, json!([{"name":"Saved"}]));
        let historical = project_historical_surface(&state).unwrap();
        assert_eq!(historical.phase, UiSurfacePhase::ReadOnly);
        assert!(historical.nodes.values().all(|node| node.bindings.is_empty() && node.actions.is_empty()));
        assert!(historical.nodes.values().any(|node| node.props.get("value") == Some(&json!("Saved"))));
        assert_eq!(state.phase, UiSurfacePhase::Ready);
        assert!(matches!(project_surface(&state), Err(UiError::InvalidListIdentity)));
    }

    #[test]
    fn stable_keys_preserve_edit_and_action_identity_across_value_changes_and_reorder() {
        let original = list(true, json!([{"id":"a","name":"A"},{"id":7,"name":"B"}]));
        let projection = project_surface(&original).unwrap();
        let field = projection.snapshot.nodes.values().find(|node| node.bindings.get("value").map(String::as_str) == Some("/items/0/name")).unwrap();
        let action = projection.snapshot.nodes.values().find(|node| node.component_id == "button"
            && projection.scopes.get(&node.instance_id).map(String::as_str) == Some("/items/0"))
            .unwrap().actions["action"].clone();
        let interaction = UiInteraction { identity:original.identity.clone(), expected_surface_revision:1,
            edits:vec![UiEdit::Set { instance_id:field.instance_id.clone(), binding:"value".into(), value:json!("Edited") }],
            action:Some(action.clone()) };
        let prepared = prepare_interaction(&original, &interaction).unwrap();
        assert_eq!(prepared.event.unwrap()["context"]["name"], "Edited");
        assert_eq!(prepared.snapshot.nodes[&field.instance_id].props["value"], "Edited");
        let mut reordered = prepared.state;
        reordered.data["items"].as_array_mut().unwrap().reverse();
        reordered.revision += 1;
        let next = project_surface(&reordered).unwrap();
        assert_eq!(next.snapshot.nodes[&field.instance_id].bindings["value"], "/items/1/name");
        assert_eq!(next.snapshot.nodes[&action.instance_id].actions["action"], action);
        let mut request = interaction;
        request.expected_surface_revision = reordered.revision;
        request.edits = vec![UiEdit::Set { instance_id:field.instance_id.clone(), binding:"value".into(), value:json!("After reorder") }];
        let result = prepare_interaction(&reordered, &request).unwrap();
        assert_eq!(result.state.data["items"][1]["name"], "After reorder");
        assert_eq!(result.state.data["items"][0]["name"], "B");
    }

    #[test]
    fn input_bindings_cannot_modify_stable_keys_or_their_parent_data() {
        for path in ["id", "/items/0/id", "/items/0", "/items", "/"] {
            let mut state = list(true, json!([{"id":"a","name":"A"}]));
            state.components.get_mut("field").unwrap()["value"] = json!({"path":path});
            assert!(matches!(project_surface(&state), Err(UiError::ImmutableListIdentity)), "{path}");
        }
    }
}
