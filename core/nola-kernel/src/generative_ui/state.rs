// SPDX-License-Identifier: Apache-2.0
//! Candidate transitions: the caller commits returned state with its receipt/journal.
use super::{data, schema, A2uiMessage};
use nola_domain::generative_ui::*;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn transition_surface(
    current: Option<&UiSurfaceState>,
    identity: &UiSurfaceIdentity,
    expected_revision: u64,
    message: A2uiMessage,
) -> Result<UiSurfaceState, UiError> {
    identity.validate()?;
    if message.surface_id() != identity.surface_id {
        return Err(UiError::StaleOwner);
    }
    let mut next = match (current, &message) {
        (None, A2uiMessage::Create(create)) => {
            if expected_revision != 0 { return Err(UiError::RevisionConflict); }
            if create.catalog_id != schema::NOLA_CATALOG_ID { return Err(UiError::UnsupportedCatalog); }
            UiSurfaceState {
                identity: identity.clone(), revision: 0,
                catalog_id: create.catalog_id.clone(), catalog_digest: schema::catalog_digest(),
                phase: UiSurfacePhase::Pending, components: BTreeMap::new(),
                data: Value::Object(Map::new()), theme: create.theme.clone(),
                send_data_model: create.send_data_model,
            }
        }
        (Some(state), _) => {
            if state.identity != *identity { return Err(UiError::StaleOwner); }
            if state.revision != expected_revision { return Err(UiError::RevisionConflict); }
            if matches!(state.phase, UiSurfacePhase::Deleted | UiSurfacePhase::ReadOnly) { return Err(UiError::SurfaceClosed); }
            if matches!(message, A2uiMessage::Create(_)) { return Err(UiError::InvalidMessage); }
            state.clone()
        }
        _ => return Err(UiError::SurfaceClosed),
    };
    match message {
        A2uiMessage::Create(_) => {}
        A2uiMessage::Components(update) => {
            let mut seen = BTreeSet::new();
            for component in update.components {
                let id = component.get("id").and_then(Value::as_str)
                    .ok_or(UiError::InvalidMessage)?.to_owned();
                if id.is_empty() || id.len() > 512 || !seen.insert(id.clone()) {
                    return Err(UiError::InvalidMessage);
                }
                next.components.insert(id, component);
            }
        }
        A2uiMessage::Data { path, mutation, .. } => next.data = data::update_data(&next.data, &path, &mutation)?,
        A2uiMessage::Delete { .. } => {
            next.phase = UiSurfacePhase::Deleted;
            next.components.clear();
            next.data = Value::Object(Map::new());
        }
    }
    if next.components.len() > UI_MAX_NODES { return Err(UiError::ResourceLimit); }
    // The 0.9.1 client-data-model schema requires each exported model to be
    // an object. Reject an incompatible update at admission, rather than
    // accepting a Surface whose next user action can never be delivered.
    if next.send_data_model && !next.data.is_object() {
        return Err(UiError::InvalidMessage);
    }
    if next.phase != UiSurfacePhase::Deleted {
        let mut completed = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        // Validate disconnected nodes too: a later reference cannot activate a
        // preexisting cycle without an admission failure.
        for id in next.components.keys() {
            graph_ready(id, &next.components, &mut visiting, &mut completed, 0)?;
        }
        let (ready, _) = graph_ready("root", &next.components, &mut BTreeSet::new(), &mut completed, 0)?;
        next.phase = if ready { UiSurfacePhase::Ready } else { UiSurfacePhase::Pending };
    }
    next.revision = expected_revision.checked_add(1).ok_or(UiError::ResourceLimit)?;
    if serde_json::to_vec(&next).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_SURFACE_BYTES {
        return Err(UiError::ResourceLimit);
    }
    Ok(next)
}

fn graph_ready(
    id: &str, components: &BTreeMap<String, Value>, visiting: &mut BTreeSet<String>,
    completed: &mut BTreeMap<String, (bool, usize)>, depth: usize,
) -> Result<(bool, usize), UiError> {
    if depth > UI_MAX_DEPTH { return Err(UiError::ResourceLimit); }
    if let Some(&(ready, height)) = completed.get(id) {
        // A subtree visited near the root may later occur on a deeper path.
        // Reuse its height, not just a boolean, to check every path's depth.
        if depth + height > UI_MAX_DEPTH { return Err(UiError::ResourceLimit); }
        return Ok((ready, height));
    }
    let Some(component) = components.get(id) else { return Ok((false, 0)); };
    if !visiting.insert(id.to_owned()) { return Err(UiError::InvalidMessage); }
    let mut ready = true;
    let mut height = 0;
    for child in child_ids(component)? {
        let (child_ready, child_height) = graph_ready(&child, components, visiting, completed, depth + 1)?;
        ready &= child_ready;
        height = height.max(child_height + 1);
    }
    visiting.remove(id);
    completed.insert(id.to_owned(), (ready, height));
    Ok((ready, height))
}

pub fn child_ids(component: &Value) -> Result<Vec<String>, UiError> {
    let mut ids = Vec::new();
    for property in ["child", "trigger", "content"] {
        if let Some(value) = component.get(property) {
            ids.push(value.as_str().ok_or(UiError::InvalidMessage)?.to_owned());
        }
    }
    if let Some(children) = component.get("children") {
        if let Some(array) = children.as_array() {
            for child in array { ids.push(child.as_str().ok_or(UiError::InvalidMessage)?.to_owned()); }
        } else {
            ids.push(children.get("componentId").and_then(Value::as_str)
                .ok_or(UiError::InvalidMessage)?.to_owned());
        }
    }
    if let Some(tabs) = component.get("tabs") {
        for tab in tabs.as_array().ok_or(UiError::InvalidMessage)? {
            ids.push(tab.get("child").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?.to_owned());
        }
    }
    Ok(ids)
}

/// One model tool call commits one surface revision, regardless of how many
/// protocol messages it contains. Intermediate candidates never escape.
pub fn transition_messages(
    current: Option<&UiSurfaceState>, identity: &UiSurfaceIdentity,
    input: &nola_domain::generative_ui_tool::UiToolInput,
) -> Result<UiSurfaceState, UiError> {
    input.validate()?;
    if current.map_or(0, |state| state.revision) != input.expected_revision {
        return Err(UiError::RevisionConflict);
    }
    let mut candidate = current.cloned();
    for message in &input.messages {
        let bytes = serde_json::to_vec(message).map_err(|_| UiError::InvalidMessage)?;
        let parsed = super::parse_a2ui_message(&bytes)?;
        let revision = candidate.as_ref().map_or(0, |state| state.revision);
        candidate = Some(transition_surface(candidate.as_ref(), identity, revision, parsed)?);
    }
    let mut result = candidate.ok_or(UiError::InvalidMessage)?;
    result.revision = input.expected_revision.checked_add(1).ok_or(UiError::ResourceLimit)?;
    Ok(result)
}

#[cfg(test)]
mod graph_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exported_model_rejects_scalar_root_without_changing_current_state() {
        let identity = UiSurfaceIdentity {
            owner: UiSurfaceOwner {
                authority_id: "host".into(), conversation_id: "conversation".into(),
                participant_id: "agent".into(), context_id: "context".into(),
                originating_run_id: "run".into(), message_id: "message".into(), part_id: "part".into(),
            }, surface_id: "form".into(), generation: 1,
        };
        let initial = transition_surface(None, &identity, 0, A2uiMessage::Create(super::super::CreateSurface {
            surface_id: "form".into(), catalog_id: schema::NOLA_CATALOG_ID.into(),
            theme: None, send_data_model: true,
        })).unwrap();
        let rejected = transition_surface(Some(&initial), &identity, 1, A2uiMessage::Data {
            surface_id: "form".into(), path: "/".into(), mutation: super::super::DataMutation::Set(json!(42)),
        });
        assert_eq!(rejected, Err(UiError::InvalidMessage));
        assert_eq!(initial.data, json!({}));
        assert_eq!(initial.revision, 1);
    }

    #[test]
    fn shared_subtree_must_fit_the_deepest_reference_path() {
        let mut components = BTreeMap::new();
        components.insert("shared".into(), json!({"component":"Card","child":"leaf"}));
        components.insert("leaf".into(), json!({"component":"Text","text":"value"}));
        let mut cache = BTreeMap::new();
        assert_eq!(graph_ready("shared", &components, &mut BTreeSet::new(), &mut cache, 0), Ok((true, 1)));
        assert_eq!(graph_ready("shared", &components, &mut BTreeSet::new(), &mut cache, UI_MAX_DEPTH), Err(UiError::ResourceLimit));
    }

    #[test]
    fn incomplete_shared_subtrees_remain_pending_and_cycles_are_rejected() {
        let mut components = BTreeMap::new();
        components.insert("root".into(), json!({"component":"Row","children":["shared","shared"]}));
        components.insert("shared".into(), json!({"component":"Card","child":"missing"}));
        assert_eq!(graph_ready("root", &components, &mut BTreeSet::new(), &mut BTreeMap::new(), 0), Ok((false, 2)));
        components.insert("missing".into(), json!({"component":"Card","child":"root"}));
        assert_eq!(graph_ready("root", &components, &mut BTreeSet::new(), &mut BTreeMap::new(), 0), Err(UiError::InvalidMessage));
    }
}
