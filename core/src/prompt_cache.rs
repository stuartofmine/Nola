// SPDX-License-Identifier: Apache-2.0
//! Native ABI adapter; all business computation is owned by Agent engine.
pub use nola_agent_engine::prompt_cache::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
#[no_mangle]
pub extern "C" fn nola_core_prompt_cache_abi_version() -> u32 {
    PROMPT_CACHE_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_prompt_cache_diagnose(
    input_json: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_json.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_json, input_len) };
    let output = match serde_json::from_slice::<PromptCacheDiagnosticInput>(bytes) {
        Ok(input) => serde_json::to_string(&diagnose_prompt_cache(&input)),
        Err(error) => serde_json::to_string(&serde_json::json!({
            "schemaVersion": PROMPT_CACHE_SCHEMA_VERSION,
            "error": "invalid_prompt_cache_diagnostic_input",
            "detail": error.to_string(),
        })),
    };
    output
        .ok()
        .and_then(|json| CString::new(json).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}
