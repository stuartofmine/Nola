// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements preprocessor logic in the Rust core module.
// - Interfaces:
//   - preprocess_events
// - APIs:
//   - none
//
// Preprocessor: Converts RawEvent -> ProcessedEvent with normalization and scoring

use crate::types::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

pub fn preprocess_events(raw: Vec<RawEvent>) -> Vec<ProcessedEvent> {
    raw.into_iter()
        .map(|re| {
            // simple priority scoring: base 0.5, bump for keywords and presence of date
            let mut score = 0.5f64;
            if re.title.to_lowercase().contains("urgent") || re.title.to_lowercase().contains("pay")
            {
                score += 0.2;
            }
            if re.date.is_some() {
                score += 0.1
            }
            if let Some(pstr) = re.metadata.get("priority") {
                if let Ok(p) = pstr.parse::<f64>() {
                    score = (score + p) / 2.0
                }
            }

            // dedupe key using parser helper
            let dedupe = crate::parser::generate_dedupe_key_internal(
                &re.source,
                &re.title,
                re.date.as_deref().unwrap_or(""),
                &re.id,
            );

            ProcessedEvent {
                id: re.id.clone(),
                source: re.source.clone(),
                source_type: re.source.clone(),
                source_id: re.id.clone(),
                title: re.title.clone(),
                date: re.date.clone(),
                start: None,
                due: None,
                priority: (score).min(1.0),
                tags: None,
                dedupe_key: dedupe,
                provenance: None,
                metadata: re.metadata.clone(),
                notes: re.notes.clone(),
                hash: crate::parser::hash_str_i64(&re.title),
            }
        })
        .collect()
}

// FFI: Preprocess raw events (JSON) -> ProcessedEvent[] JSON
#[no_mangle]
pub extern "C" fn nola_core_preprocess_raw_events(
    data_ptr: *const u8,
    data_len: usize,
) -> *mut c_char {
    if data_ptr.is_null() || data_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        if let Ok(s) = std::str::from_utf8(slice) {
            if let Ok(raw_events) = serde_json::from_str::<Vec<RawEvent>>(s) {
                let processed = preprocess_events(raw_events);
                if let Ok(out) = serde_json::to_string(&processed) {
                    return CString::new(out).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}
