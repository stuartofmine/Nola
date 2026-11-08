// SPDX-License-Identifier: Apache-2.0
//! Single-version A2UI framing. This module performs no I/O or side effects.

use nola_domain::generative_ui::{UiError, A2UI_WIRE_VERSION, UI_MAX_MESSAGE_BYTES, UI_MAX_NODES};
use serde::Deserialize;
use serde_json::Value;

pub mod data;
pub mod date;
pub mod schema;
pub mod state;
pub mod resolve;
pub mod projection;
pub mod interaction;
pub mod metadata;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSurface {
    pub surface_id: String,
    pub catalog_id: String,
    pub theme: Option<Value>,
    #[serde(default)]
    pub send_data_model: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateComponents {
    pub surface_id: String,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataMutation {
    Set(Value),
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum A2uiMessage {
    Create(CreateSurface),
    Components(UpdateComponents),
    Data {
        surface_id: String,
        path: String,
        mutation: DataMutation,
    },
    Delete { surface_id: String },
}

impl A2uiMessage {
    pub fn surface_id(&self) -> &str {
        match self {
            Self::Create(value) => &value.surface_id,
            Self::Components(value) => &value.surface_id,
            Self::Data { surface_id, .. } | Self::Delete { surface_id } => surface_id,
        }
    }
}

/// Parse only a complete, bounded transport message. Catalog schema validation
/// and graph admission must follow before any state is committed.
pub fn parse_a2ui_message(bytes: &[u8]) -> Result<A2uiMessage, UiError> {
    if bytes.len() > UI_MAX_MESSAGE_BYTES {
        return Err(UiError::ResourceLimit);
    }
    let value = crate::strict_json::decode(bytes).map_err(|_| UiError::InvalidMessage)?;
    let envelope = value.as_object().ok_or(UiError::InvalidMessage)?;
    if envelope.get("version").and_then(Value::as_str) != Some(A2UI_WIRE_VERSION) {
        return Err(UiError::UnsupportedProtocol);
    }
    if envelope.len() != 2 {
        return Err(UiError::InvalidMessage);
    }
    data::check_data_budget(&value)?;
    schema::validate_message(&value)?;
    let message = if let Some(payload) = envelope.get("createSurface") {
        A2uiMessage::Create(
            serde_json::from_value(payload.clone()).map_err(|_| UiError::InvalidMessage)?,
        )
    } else if let Some(payload) = envelope.get("updateComponents") {
        let update: UpdateComponents =
            serde_json::from_value(payload.clone()).map_err(|_| UiError::InvalidMessage)?;
        if update.components.is_empty() {
            return Err(UiError::InvalidMessage);
        }
        if update.components.len() > UI_MAX_NODES {
            return Err(UiError::ResourceLimit);
        }
        for component in &update.components {
            validate_component_functions(component)?;
            let name = component.get("component").and_then(Value::as_str).ok_or(UiError::InvalidMessage)?;
            if !schema::SUPPORTED_COMPONENTS.contains(&name) { return Err(UiError::UnsupportedComponent); }
            // The advertised Nola catalog currently exposes event actions.
            // Never publish an executable-looking native function that the
            // trusted Host capability boundary cannot yet dispatch.
            if component.get("action").is_some_and(|action| action.get("functionCall").is_some()) {
                return Err(UiError::UnsupportedFunction);
            }
        }
        A2uiMessage::Components(update)
    } else if let Some(payload) = envelope.get("updateDataModel") {
        let data = payload.as_object().ok_or(UiError::InvalidMessage)?;
        if data.keys().any(|key| !matches!(key.as_str(), "surfaceId" | "path" | "value")) {
            return Err(UiError::InvalidMessage);
        }
        let surface_id = data.get("surfaceId").and_then(Value::as_str)
            .ok_or(UiError::InvalidMessage)?.to_owned();
        let path = match data.get("path") {
            None => "/".to_owned(),
            Some(Value::String(path)) => path.clone(),
            _ => return Err(UiError::InvalidMessage),
        };
        // Option<Value> deserialization would erase null vs missing.
        let mutation = match data.get("value") {
            Some(value) => DataMutation::Set(value.clone()),
            None => DataMutation::Delete,
        };
        A2uiMessage::Data { surface_id, path, mutation }
    } else if let Some(payload) = envelope.get("deleteSurface") {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Delete { surface_id: String }
        let deletion: Delete =
            serde_json::from_value(payload.clone()).map_err(|_| UiError::InvalidMessage)?;
        A2uiMessage::Delete { surface_id: deletion.surface_id }
    } else {
        return Err(UiError::InvalidMessage);
    };
    if message.surface_id().trim().is_empty() || message.surface_id().len() > 512 {
        return Err(UiError::InvalidIdentity);
    }
    Ok(message)
}

/// Check the complete admitted graph, including nodes not reachable yet. A
/// pending Surface must not hide unsupported calls until a later activation.
fn validate_component_functions(value: &Value) -> Result<(), UiError> {
    match value {
        Value::Object(fields) => {
            if let Some(call) = fields.get("call") {
                let name = call.as_str().ok_or(UiError::InvalidMessage)?;
                if !schema::SUPPORTED_FUNCTIONS.contains(&name) {
                    return Err(UiError::UnsupportedFunction);
                }
            }
            for child in fields.values() { validate_component_functions(child)?; }
        }
        Value::Array(values) => for child in values { validate_component_functions(child)?; },
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod contract_tests;
