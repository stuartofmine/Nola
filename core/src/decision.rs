// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements decision logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
// Decision engine logic

use crate::parser::hash_str_i64;
use crate::types::*;
use chrono::Utc;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

/// Decide actions based on ProcessedEvent[] and ModelOutput
#[no_mangle]
pub extern "C" fn nola_core_decide_actions(
    processed_ptr: *const u8,
    processed_len: usize,
    model_ptr: *const u8,
    model_len: usize,
) -> *mut c_char {
    if processed_ptr.is_null() || processed_len == 0 || model_ptr.is_null() || model_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let proc_slice = std::slice::from_raw_parts(processed_ptr, processed_len);
        let model_slice = std::slice::from_raw_parts(model_ptr, model_len);
        if let (Ok(proc_str), Ok(model_str)) = (
            std::str::from_utf8(proc_slice),
            std::str::from_utf8(model_slice),
        ) {
            let events_res = serde_json::from_str::<Vec<ProcessedEvent>>(proc_str);
            if let Err(e) = events_res {
                if let Ok(err_json) = serde_json::to_string(&serde_json::json!({
                    "error": format!("processed parse failed: {}", e.to_string())
                })) {
                    return CString::new(err_json).unwrap().into_raw();
                }
                return ptr::null_mut();
            }
            let model_res = serde_json::from_str::<ModelOutputRust>(model_str);
            if let Err(e) = model_res {
                if let Ok(err_json) = serde_json::to_string(&serde_json::json!({
                    "error": format!("model parse failed: {}", e.to_string())
                })) {
                    return CString::new(err_json).unwrap().into_raw();
                }
                return ptr::null_mut();
            }
            let events = events_res.unwrap();
            let model = model_res.unwrap();

            // Build map of event by dedupe_key
            let mut map: std::collections::HashMap<String, ProcessedEvent> =
                std::collections::HashMap::new();
            for e in events.into_iter() {
                map.insert(e.dedupe_key.clone(), e);
            }

            let mut candidates: Vec<NotificationCandidate> = vec![];
            let items_clone = model.items.clone();
            for it in items_clone.into_iter() {
                if let Some(ev) = map.get(&it.dedupe_key) {
                    let cand = NotificationCandidate {
                        event: ev.clone(),
                        dedupe_key: it.dedupe_key.clone(),
                        text: it.text.clone(),
                        action_type: it
                            .action
                            .clone()
                            .unwrap_or_else(|| String::from("openCalendar")),
                    };
                    candidates.push(cand);
                }
            }

            let mut res = DecisionResult {
                save_summary: false,
                summary_text: None,
                summary_event: None,
                candidates,
            };
            if !model.items.is_empty()
                || (model.explain.is_some() && !model.explain.as_ref().unwrap().trim().is_empty())
            {
                res.save_summary = true;
                res.summary_text = model.explain.clone();
                if model.items.is_empty() {
                    // create summary event
                    let text = model.explain.clone().unwrap_or_else(|| String::from(""));
                    let pe = ProcessedEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        source: String::from("reminder"),
                        source_type: String::from("manual"),
                        source_id: String::from("processing_agent_summary"),
                        title: text.clone(),
                        date: None,
                        start: None,
                        due: None,
                        priority: 0.5,
                        tags: None,
                        dedupe_key: format!(
                            "summary:{}:{}",
                            Utc::now().date_naive().format("%Y-%m-%d"),
                            format!("{:x}", md5::compute(&text))
                        ),
                        provenance: None,
                        metadata: std::collections::HashMap::new(),
                        notes: None,
                        hash: hash_str_i64(&text),
                    };
                    res.summary_event = Some(pe);
                }
            }

            if let Ok(out) = serde_json::to_string(&res) {
                return CString::new(out).unwrap().into_raw();
            }
        }
    }
    ptr::null_mut()
}
