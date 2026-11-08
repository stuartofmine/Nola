// SPDX-License-Identifier: Apache-2.0
//! Native Agent ABI adapters over the uniquely owned engine.
pub use nola_agent_engine::agent::*;

pub mod prompt_composer {
pub use nola_agent_engine::agent::prompt_composer::*;
use serde::{Deserialize, Serialize};
use std::ffi::{CString, c_char};
#[derive(Debug, Deserialize)]
struct AgentPromptComposeABIRequest {
    schema_version: u32,
    fragments: Vec<AgentPromptFragment>,
}

#[derive(Debug, Serialize)]
struct AgentPromptComposeABIError {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AgentPromptComposeABIResponse {
    schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    composition: Option<AgentPromptComposition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<AgentPromptComposeABIError>,
}

#[no_mangle]
pub extern "C" fn nola_core_prompt_composer_abi_version() -> u32 {
    AGENT_PROMPT_COMPOSER_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_compose_agent_prompt_json(
    input_json: *const u8,
    input_len: usize,
) -> *mut c_char {
    let response = if input_json.is_null() {
        compose_abi_error("invalid_input", "Prompt composer input pointer is null")
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(input_json, input_len) };
        match serde_json::from_slice::<AgentPromptComposeABIRequest>(bytes) {
            Ok(request) if request.schema_version == AGENT_PROMPT_COMPOSER_ABI_VERSION => {
                match compose_host_agent_prompt(request.fragments) {
                    Ok(composition) => AgentPromptComposeABIResponse {
                        schema_version: AGENT_PROMPT_COMPOSER_ABI_VERSION,
                        composition: Some(composition),
                        error: None,
                    },
                    Err(error) => prompt_error_response(error),
                }
            }
            Ok(request) => compose_abi_error(
                "incompatible_schema",
                format!(
                    "Unsupported prompt composer schema version: {}",
                    request.schema_version
                ),
            ),
            Err(error) => compose_abi_error("invalid_json", error.to_string()),
        }
    };
    let json = serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"schema_version":1,"error":{"code":"serialization_failed","message":"Prompt composer response serialization failed"}}"#.to_string()
    });
    CString::new(json).map_or(std::ptr::null_mut(), CString::into_raw)
}

fn prompt_error_response(error: PromptCompositionError) -> AgentPromptComposeABIResponse {
    match error {
        PromptCompositionError::TooManyFragments { count, limit } => compose_abi_error(
            "too_many_fragments",
            format!("Prompt has {count} fragments; maximum is {limit}"),
        ),
        PromptCompositionError::FragmentTooLarge { id, chars, limit } => compose_abi_error(
            "fragment_too_large",
            format!("Prompt fragment {id} has {chars} characters; maximum is {limit}"),
        ),
        PromptCompositionError::PromptContentTooLarge { chars, limit } => compose_abi_error(
            "prompt_content_too_large",
            format!("Prompt fragments contain {chars} characters; maximum is {limit}"),
        ),
        PromptCompositionError::EmptyId => {
            compose_abi_error("empty_id", "Prompt fragment id is empty")
        }
        PromptCompositionError::DuplicateId(id) => compose_abi_error("duplicate_id", id),
        PromptCompositionError::HostPolicyFragment(id) => {
            compose_abi_error("host_policy_fragment", id)
        }
        PromptCompositionError::EmptyPolicyFragment(id) => {
            compose_abi_error("empty_policy_fragment", id)
        }
        PromptCompositionError::ReferenceMarkedCacheable(id) => {
            compose_abi_error("reference_marked_cacheable", id)
        }
        PromptCompositionError::RuntimeStateMarkedCacheable(id) => {
            compose_abi_error("runtime_state_marked_cacheable", id)
        }
        PromptCompositionError::RuntimeStateInvalidTrust(id) => {
            compose_abi_error("runtime_state_invalid_trust", id)
        }
    }
}

fn compose_abi_error(
    code: &'static str,
    message: impl Into<String>,
) -> AgentPromptComposeABIResponse {
    AgentPromptComposeABIResponse {
        schema_version: AGENT_PROMPT_COMPOSER_ABI_VERSION,
        composition: None,
        error: Some(AgentPromptComposeABIError {
            code,
            message: message.into(),
        }),
    }
}


}

pub mod resource_access {
pub use nola_agent_engine::agent::resource_access::*;
use serde::Serialize;
use std::ffi::{CString, c_char};
use std::ptr;
#[derive(Debug, Serialize)]
struct ResourceAccessABIResponse {
    schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<AgentResourceAccessDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ResourceAccessABIError>,
}

#[derive(Debug, Serialize)]
struct ResourceAccessABIError {
    code: &'static str,
    message: String,
}

#[no_mangle]
pub extern "C" fn nola_core_resource_access_abi_version() -> u32 {
    RESOURCE_ACCESS_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_resource_access_evaluate_json(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    let response = if input_ptr.is_null() || input_len == 0 {
        resource_access_error("invalid_input", "Resource access input is empty")
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
        match serde_json::from_slice::<AgentResourceAccessRequest>(bytes) {
            Ok(request) if request.schema_version == RESOURCE_ACCESS_SCHEMA_VERSION => {
                ResourceAccessABIResponse {
                    schema_version: RESOURCE_ACCESS_SCHEMA_VERSION,
                    decision: Some(evaluate_resource_access(&request)),
                    error: None,
                }
            }
            Ok(request) => resource_access_error(
                "incompatible_schema",
                format!(
                    "Unsupported resource access schema: {}",
                    request.schema_version
                ),
            ),
            Err(error) => resource_access_error("invalid_json", error.to_string()),
        }
    };
    serde_json::to_string(&response)
        .ok()
        .and_then(|json| CString::new(json).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

fn resource_access_error(
    code: &'static str,
    message: impl Into<String>,
) -> ResourceAccessABIResponse {
    ResourceAccessABIResponse {
        schema_version: RESOURCE_ACCESS_SCHEMA_VERSION,
        decision: None,
        error: Some(ResourceAccessABIError {
            code,
            message: message.into(),
        }),
    }
}

}
