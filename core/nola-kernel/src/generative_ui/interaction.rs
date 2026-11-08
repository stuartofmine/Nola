// SPDX-License-Identifier: Apache-2.0
//! Atomic edit + action preparation. Host execution still requires existing admission.
use super::{data::{overlapping_paths, update_data_batch}, projection::project_surface, resolve::Evaluator, DataMutation};
use nola_domain::generative_ui::*;
use serde_json::Value;

pub struct PreparedUiInteraction {
    pub state: UiSurfaceState,
    pub snapshot: UiSnapshot,
    /// Resolved event input, not an authorization and never an executable tool call.
    pub event: Option<Value>,
}

pub fn prepare_interaction(state: &UiSurfaceState, interaction: &UiInteraction) -> Result<PreparedUiInteraction, UiError> {
    if state.identity != interaction.identity { return Err(UiError::StaleOwner); }
    if state.revision != interaction.expected_surface_revision { return Err(UiError::RevisionConflict); }
    if state.phase != UiSurfacePhase::Ready { return Err(UiError::SurfaceClosed); }
    if serde_json::to_vec(interaction).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_ACTION_BYTES {
        return Err(UiError::ResourceLimit);
    }
    let original = project_surface(state)?;
    let mut candidate = state.clone();
    let mut edited_paths = std::collections::BTreeSet::new();
    let mut assigned_instances = Vec::new();
    let mut mutations = Vec::new();
    for edit in &interaction.edits {
        let (instance, binding, mutation) = match edit {
            UiEdit::Set { instance_id, binding, value } => (instance_id, binding, DataMutation::Set(value.clone())),
            UiEdit::Delete { instance_id, binding } => (instance_id, binding, DataMutation::Delete),
        };
        let node = original.snapshot.nodes.get(instance).ok_or(UiError::StaleOwner)?;
        let path = node.bindings.get(binding).ok_or(UiError::InvalidBinding)?;
        if node.props.get("disabled").and_then(Value::as_bool) == Some(true)
            || binding != "value"
            || !matches!(node.component.as_str(), "TextField" | "DateTimeInput" | "CheckBox" | "Slider" | "ChoicePicker")
            || edited_paths.iter().any(|previous: &String| overlapping_paths(previous, path)) {
            return Err(UiError::InvalidBinding);
        }
        edited_paths.insert(path.clone());
        if let DataMutation::Set(value) = &mutation { assigned_instances.push((instance, value.clone())); }
        // The client chooses an admitted binding name, never an arbitrary data path.
        mutations.push((super::data::pointer_tokens(path)?, path.clone(), mutation));
    }
    // Every binding above was resolved against the same original snapshot.
    // Set values before deleting, then remove array siblings from the highest
    // index down so a deletion cannot redirect another edit to a different row.
    mutations.sort_by(|(left_tokens, _, left), (right_tokens, _, right)| {
        let deletion = |mutation: &DataMutation| matches!(mutation, DataMutation::Delete);
        deletion(left).cmp(&deletion(right)).then_with(|| {
            if deletion(left) {
                for (left, right) in left_tokens.iter().zip(right_tokens) {
                    let order = match (left.parse::<usize>(), right.parse::<usize>()) {
                        (Ok(left), Ok(right)) => right.cmp(&left),
                        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                        _ => right.cmp(left),
                    };
                    if !order.is_eq() { return order; }
                }
                right_tokens.len().cmp(&left_tokens.len())
            } else { std::cmp::Ordering::Equal }
        })
    });
    candidate.data = update_data_batch(&candidate.data,
        mutations.iter().map(|(_, path, mutation)| (path.as_str(), mutation)))?;
    if candidate.send_data_model && !candidate.data.is_object() {
        return Err(UiError::InvalidMessage);
    }
    let projected = project_surface(&candidate)?;
    for (instance, value) in assigned_instances {
        let node = projected.snapshot.nodes.get(instance).ok_or(UiError::StaleOwner)?;
        validate_edit_value(node, &value)?;
    }
    // Validate the final transaction state, not intermediate edits. A batch can
    // change a constraint and its value together; unchanged values can also
    // become invalid when a bound constraint changes.
    for node in projected.snapshot.nodes.values() {
        if node.bindings.contains_key("value") {
            if let Some(value) = node.props.get("value").filter(|value| !value.is_null()) {
                validate_edit_value(node, value)?;
            }
        }
    }
    let mut event = None;
    if let Some(action) = &interaction.action {
        let node = projected.snapshot.nodes.get(&action.instance_id).ok_or(UiError::StaleOwner)?;
        if node.props.get("disabled").and_then(Value::as_bool) == Some(true) { return Err(UiError::InvalidBinding); }
        if node.actions.get(&action.action_id) != Some(action) { return Err(UiError::InvalidMessage); }
        let fields: Vec<_> = projected.snapshot.nodes.values()
            .filter(|node| !node.validation_errors.is_empty())
            .map(|node| UiFieldValidationError {
                instance_id: node.instance_id.clone(),
                label: node.props.get("label").and_then(Value::as_str).unwrap_or("").to_owned(),
                messages: node.validation_errors.clone(),
            }).collect();
        if !fields.is_empty() { return Err(UiError::ValidationFailed { fields }); }
        let spec = candidate.components.get(&node.component_id).ok_or(UiError::StaleOwner)?;
        let definition = spec.get(&action.action_id).ok_or(UiError::InvalidMessage)?;
        let scope = projected.scopes.get(&action.instance_id).ok_or(UiError::StaleOwner)?;
        let mut evaluator = Evaluator::new(&candidate.data);
        if let Some(value) = definition.get("event") {
            // Only the event context is dynamic. Event names are literal catalog input.
            let mut resolved = value.clone();
            if let Some(context) = value.get("context") { resolved["context"] = evaluator.resolve(context, scope)?; }
            else { resolved["context"] = serde_json::json!({}); }
            resolved["surfaceId"] = Value::String(state.identity.surface_id.clone());
            resolved["sourceComponentId"] = Value::String(node.component_id.clone());
            event = Some(resolved);
        } else { return Err(UiError::UnsupportedFunction); }
    }
    if let Some(resolved) = &event {
        if serde_json::to_vec(resolved).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_ACTION_BYTES {
            return Err(UiError::ResourceLimit);
        }
    }
    candidate.revision = candidate.revision.checked_add(1).ok_or(UiError::ResourceLimit)?;
    if serde_json::to_vec(&candidate).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_SURFACE_BYTES {
        return Err(UiError::ResourceLimit);
    }
    let mut snapshot = projected.snapshot;
    snapshot.revision = candidate.revision;
    Ok(PreparedUiInteraction { state: candidate, snapshot, event })
}

fn validate_edit_value(node: &UiNodeSnapshot, value: &Value) -> Result<(), UiError> {
    let valid = match node.component.as_str() {
        // v0.9.1 TextField.value is DynamicString even for the numeric variant.
        "TextField" => value.is_string(),
        "DateTimeInput" => value.as_str().is_some_and(|text| super::date::validate(node, text).is_ok()),
        "CheckBox" => value.is_boolean(),
        "Slider" => value.as_f64().is_some_and(|number| {
            let min = node.props.get("min").and_then(Value::as_f64).unwrap_or(0.0);
            let max = node.props.get("max").and_then(Value::as_f64);
            number.is_finite() && number >= min && max.is_some_and(|max| number <= max)
        }),
        "ChoicePicker" => value.as_array().is_some_and(|values| {
            let multiple = node.props.get("variant").and_then(Value::as_str) == Some("multipleSelection");
            let Some(options) = node.props.get("options").and_then(Value::as_array) else { return false; };
            let mut seen = std::collections::BTreeSet::new();
            (multiple || values.len() <= 1) && values.iter().all(|value| {
                value.as_str().is_some_and(|text| seen.insert(text))
                    && options.iter().any(|option| option.get("value") == Some(value))
            })
        }),
        _ => false,
    };
    if valid { Ok(()) } else {
        Err(UiError::ValidationFailed { fields: vec![UiFieldValidationError {
            instance_id: node.instance_id.clone(),
            label: node.props.get("label").and_then(Value::as_str).unwrap_or("").to_owned(),
            messages: vec![match node.component.as_str() {
                "ChoicePicker" => "Choose only available options and respect the selection limit.",
                "Slider" => "Choose a number within the allowed range.",
                "DateTimeInput" => "Choose a valid date or time within the allowed range.",
                "CheckBox" => "The checkbox value must be true or false.",
                _ => "The field value must be text.",
            }.into()],
        }] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> UiSurfaceState {
        serde_json::from_value(json!({
            "identity": {"owner": {"authority_id":"host", "conversation_id":"chat",
                "participant_id":"assistant", "context_id":"chat", "originating_run_id":"run",
                "message_id":"message", "part_id":"part"}, "surface_id":"form", "generation":1},
            "revision":1, "catalog_id":super::super::schema::NOLA_CATALOG_ID,
            "catalog_digest":super::super::schema::catalog_digest(), "phase":"ready",
            "components": {
                "root":{"id":"root","component":"Column","children":["limit","amount"]},
                "limit":{"id":"limit","component":"Slider","min":0,"max":100,"value":{"path":"/limit"}},
                "amount":{"id":"amount","component":"Slider","min":0,"max":{"path":"/limit"},"value":{"path":"/amount"}}
            }, "data":{"limit":10,"amount":5}, "theme":null,"send_data_model":false
        })).unwrap()
    }

    fn interaction(state: &UiSurfaceState, changes: &[(&str, Value)]) -> UiInteraction {
        let projection = project_surface(state).unwrap();
        UiInteraction { identity:state.identity.clone(), expected_surface_revision:state.revision,
            action:None, edits:changes.iter().map(|(component, value)| {
                let node = projection.snapshot.nodes.values().find(|node| node.component_id == *component).unwrap();
                UiEdit::Set { instance_id:node.instance_id.clone(), binding:"value".into(), value:value.clone() }
            }).collect() }
    }

    #[test]
    fn dependent_constraint_edits_are_atomic_and_order_independent() {
        let state = form();
        for changes in [vec![("amount",json!(15)),("limit",json!(20))],
                        vec![("limit",json!(20)),("amount",json!(15))]] {
            let result = prepare_interaction(&state, &interaction(&state, &changes)).unwrap();
            assert_eq!(result.state.data, json!({"limit":20,"amount":15}));
            assert_eq!(result.state.revision, 2);
        }
        assert_eq!(state.data, json!({"limit":10,"amount":5}));
    }

    #[test]
    fn changing_constraint_cannot_invalidate_an_unchanged_value() {
        let state = form();
        assert!(prepare_interaction(&state, &interaction(&state, &[("limit",json!(3))])).is_err());
        assert!(prepare_interaction(&state, &interaction(&state, &[("amount",Value::Null)])).is_err());
        assert_eq!(state.data, json!({"limit":10,"amount":5}));
    }
    fn array_form() -> UiSurfaceState {
        let mut state = form();
        state.components.clear();
        state.components.insert("root".into(), json!({"id":"root","component":"Column","children":["first","second","third"]}));
        for (index, id) in ["first", "second", "third"].iter().enumerate() {
            state.components.insert((*id).into(), json!({"id":id,"component":"CheckBox",
                "label":id,"value":{"path":format!("/items/{index}")}}));
        }
        state.data = json!({"items":[false,false,true,false]});
        state
    }

    #[test]
    fn array_deletion_cannot_redirect_another_bound_edit() {
        let state = array_form();
        let mut request = interaction(&state, &[("first", json!(false)), ("second", json!(true))]);
        let UiEdit::Set { instance_id, binding, .. } = request.edits.remove(0) else { unreachable!() };
        request.edits.insert(0, UiEdit::Delete { instance_id, binding });
        for reverse in [false, true] {
            if reverse { request.edits.reverse(); }
            let prepared = prepare_interaction(&state, &request).unwrap();
            assert_eq!(prepared.state.data, json!({"items":[true,true,false]}));
        }
        assert_eq!(state.data, json!({"items":[false,false,true,false]}));
    }

    #[test]
    fn deletion_cannot_hide_an_invalid_assignment_at_the_original_binding() {
        let state = array_form();
        let mut request = interaction(&state, &[("first", json!(false)), ("second", json!("not a boolean"))]);
        let UiEdit::Set { instance_id, binding, .. } = request.edits.remove(0) else { unreachable!() };
        request.edits.insert(0, UiEdit::Delete { instance_id, binding });
        assert!(matches!(prepare_interaction(&state, &request), Err(UiError::ValidationFailed { .. })));
        assert_eq!(state.data, json!({"items":[false,false,true,false]}));
    }

    #[test]
    fn sibling_array_deletes_use_original_indices_in_either_order() {
        let state = array_form();
        let mut request = interaction(&state, &[("first", json!(false)), ("third", json!(true))]);
        request.edits = request.edits.into_iter().map(|edit| match edit {
            UiEdit::Set { instance_id, binding, .. } => UiEdit::Delete { instance_id, binding },
            _ => unreachable!(),
        }).collect();
        for reverse in [false, true] {
            if reverse { request.edits.reverse(); }
            let prepared = prepare_interaction(&state, &request).unwrap();
            assert_eq!(prepared.state.data, json!({"items":[false,false]}));
        }
    }

    #[test]
    fn array_delete_order_compares_numeric_indices_not_lexical_paths() {
        let mut state = array_form();
        state.data = json!({"items":(0..12).map(|index| index % 2 == 0).collect::<Vec<_>>()});
        state.components.get_mut("first").unwrap()["value"] = json!({"path":"/items/2"});
        state.components.get_mut("second").unwrap()["value"] = json!({"path":"/items/10"});
        let mut request = interaction(&state, &[("first",json!(true)),("second",json!(true))]);
        request.edits = request.edits.into_iter().map(|edit| match edit {
            UiEdit::Set { instance_id, binding, .. } => UiEdit::Delete { instance_id, binding },
            _ => unreachable!(),
        }).collect();
        let prepared = prepare_interaction(&state, &request).unwrap();
        assert_eq!(prepared.state.data, json!({"items":(0..12).filter(|index| ![2,10].contains(index))
            .map(|index| index % 2 == 0).collect::<Vec<_>>()}));
    }

    #[test]
    fn editing_exported_model_root_cannot_commit_a_scalar() {
        let mut state = form();
        state.components.clear();
        state.components.insert("root".into(), json!({"id":"root","component":"TextField",
            "label":"Root", "value":{"path":"/"}}));
        state.send_data_model = true;
        let request = interaction(&state, &[("root",json!("scalar"))]);
        assert!(matches!(prepare_interaction(&state, &request), Err(UiError::InvalidMessage)));
        assert!(state.data.is_object());
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn resolved_action_context_obeys_the_action_budget() {
        let mut state = form();
        state.components.clear();
        state.components.insert("root".into(), json!({"id":"root","component":"Button","child":"label",
            "action":{"event":{"name":"submit","context":{"payload":{"path":"/payload"}}}}}));
        state.components.insert("label".into(), json!({"id":"label","component":"Text","text":"Submit"}));
        state.data = json!({"payload":"x".repeat(UI_MAX_ACTION_BYTES + 1)});
        let projection = project_surface(&state).unwrap();
        let action = projection.snapshot.nodes.values().find_map(|node| node.actions.get("action")).unwrap().clone();
        let request = UiInteraction { identity:state.identity.clone(), expected_surface_revision:state.revision,
            edits:vec![], action:Some(action) };
        assert!(matches!(prepare_interaction(&state, &request), Err(UiError::ResourceLimit)));
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn repeated_bindings_cannot_expand_past_projection_budget() {
        let mut state = form();
        state.components.clear();
        state.components.insert("root".into(), json!({"id":"root","component":"Column","children":vec!["text"; 40]}));
        state.components.insert("text".into(), json!({"id":"text","component":"Text","text":{"path":"/payload"}}));
        state.data = json!({"payload":"x".repeat(30_000)});
        assert!(serde_json::to_vec(&state).unwrap().len() < UI_MAX_SURFACE_BYTES);
        assert!(matches!(project_surface(&state), Err(UiError::ResourceLimit)));
    }

}
