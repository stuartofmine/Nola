// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements transformer logic in the Rust core module.
// - Interfaces:
//   - transform_model_output
// - APIs:
//   - none
//
// Transformer: convert LLM ModelOutput into ProcessedEvent list

use crate::types::*;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

pub fn transform_model_output(
    model: ModelOutputRust,
    source_id: &str,
    source_type: &str,
) -> Vec<ProcessedEvent> {
    model
        .items
        .into_iter()
        .map(|it| {
            let text = it.text.clone();
            let dedupe = it.dedupe_key.clone();
            ProcessedEvent {
                id: it.dedupe_key.clone(),
                source: source_type.to_string(),
                source_type: source_type.to_string(),
                source_id: source_id.to_string(),
                title: text.clone(),
                date: None,
                start: None,
                due: None,
                priority: it.priority,
                tags: None,
                dedupe_key: dedupe,
                provenance: None,
                metadata: std::collections::HashMap::new(),
                notes: None,
                hash: crate::parser::hash_str_i64(&text),
            }
        })
        .collect()
}

#[no_mangle]
pub extern "C" fn nola_core_transform_model_output(
    output_ptr: *const u8,
    output_len: usize,
    source_id: *const c_char,
    source_type: *const c_char,
) -> *mut c_char {
    if output_ptr.is_null() || output_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(output_ptr, output_len);
        let src_id = if source_id.is_null() {
            ""
        } else {
            CStr::from_ptr(source_id).to_str().unwrap_or("")
        };
        let src_type = if source_type.is_null() {
            ""
        } else {
            CStr::from_ptr(source_type).to_str().unwrap_or("")
        };
        if let Ok(s) = std::str::from_utf8(slice) {
            if let Ok(model) = serde_json::from_str::<ModelOutputRust>(s) {
                let events = transform_model_output(model, src_id, src_type);
                if let Ok(out) = serde_json::to_string(&events) {
                    return CString::new(out).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}
