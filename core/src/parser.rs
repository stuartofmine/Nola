// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements parser logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
// URL parsing and raw event processing

use crate::types::*;
use chrono::{DateTime, Datelike};
use regex::Regex;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

/// Parse URL-fetched JSON and convert to ProcessedEvent[].
pub(crate) fn parse_url_json_internal(data: &[u8]) -> Result<Vec<ProcessedEvent>, String> {
    let s = std::str::from_utf8(data).map_err(|e| format!("UTF-8 error: {}", e))?;
    let resp: URLDataResponse =
        serde_json::from_str(s).map_err(|e| format!("JSON parse error: {}", e))?;

    let mut events = vec![];
    for je in resp.events {
        let dt = DateTime::parse_from_rfc3339(&je.date)
            .or_else(|_| DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", je.date)))
            .ok();

        let priority = je.priority.unwrap_or(0.5);
        let tags = je.metadata.as_ref().and_then(|m| m.tags.clone());
        let loc = je.metadata.as_ref().and_then(|m| m.location.clone());

        let mut meta = std::collections::HashMap::new();
        if let Some(l) = loc {
            meta.insert("location".to_string(), l);
        }

        let title = je.title.clone();
        let dedupe_key = generate_dedupe_key_internal(
            &je.r#type,
            &title,
            dt.as_ref().map(|d| d.to_rfc3339()).as_deref().unwrap_or(""),
            &je.id,
        );

        let pe = ProcessedEvent {
            id: je.id.clone(),
            source: je.r#type.clone(),
            source_type: je.r#type.clone(),
            source_id: je.id.clone(),
            title: title.clone(),
            date: dt.map(|d| d.to_rfc3339()),
            start: None,
            due: None,
            priority,
            tags,
            dedupe_key,
            provenance: None,
            metadata: meta,
            notes: je.description.clone(),
            hash: hash_str_i64(&title),
        };
        events.push(pe);
    }
    Ok(events)
}

/// Very small command parser: attempts to extract a task-like structure from free text.
/// Returns JSON string of ParsedTask.
#[no_mangle]
pub extern "C" fn nola_core_parse_command(cmd_ptr: *const c_char, cmd_len: usize) -> *mut c_char {
    if cmd_ptr.is_null() || cmd_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(cmd_ptr as *const u8, cmd_len);
        if let Ok(s) = std::str::from_utf8(slice) {
            let text = s.trim();

            // Basic heuristics
            let lower = text.to_lowercase();
            let mut task_type = "custom".to_string();
            let mut is_recurring = None;
            let mut recurrence_rule = None;
            let mut target_date = None;
            let mut related_person = None;
            let mut amount: Option<f64> = None;
            let mut currency: Option<String> = None;

            if lower.contains("birthday") || lower.contains("生日") {
                task_type = "birthday".to_string();
            } else if lower.contains("remind") || lower.contains("提醒") || lower.contains("提醒我")
            {
                task_type = "reminder".to_string();
            } else if lower.contains("debt") || lower.contains("欠") || lower.contains("借") {
                task_type = "debt".to_string();
            }

            // recurrence detection (简单)
            if lower.contains("每天") || lower.contains("daily") {
                is_recurring = Some(true);
                recurrence_rule = Some("FREQ=DAILY".to_string());
            }

            // date extraction: yyyy-mm-dd or chinese like 8月7日
            let re_iso = Regex::new(r"(\d{4}-\d{1,2}-\d{1,2})").unwrap();
            if let Some(cap) = re_iso.captures(&lower) {
                target_date = Some(cap[1].to_string());
            } else {
                // try simple month/day like 8月7日
                let re_md = Regex::new(r"(\d{1,2})月(\d{1,2})日").unwrap();
                if let Some(cap) = re_md.captures(&lower) {
                    let year = chrono::Utc::now().year();
                    let mm = &cap[1];
                    let dd = &cap[2];
                    target_date = Some(format!("{}-{:0>2}-{:0>2}", year, mm, dd));
                }
            }

            // related person detection: naive "小明" pattern (中文 names) or words after 'to'/'给'
            let re_person = Regex::new(r"给\s*([\p{Han}]{2,4})|to\s+(\w{2,20})").unwrap();
            if let Some(cap) = re_person.captures(&text) {
                if let Some(m) = cap.get(1) {
                    related_person = Some(m.as_str().to_string())
                } else if let Some(m) = cap.get(2) {
                    related_person = Some(m.as_str().to_string())
                }
            }

            // amount extraction: numbers with optional currency
            let re_amount = Regex::new(r"(\d+[\.,]?\d*)\s*(¥|￥|rmb|usd|cny|\$)?").unwrap();
            if let Some(cap) = re_amount.captures(&lower) {
                if let Ok(v) = cap[1].replace(",", ".").parse::<f64>() {
                    amount = Some(v)
                }
                if let Some(m) = cap.get(2) {
                    currency = Some(m.as_str().to_string())
                }
            }

            let parsed = ParsedTask {
                task_type,
                title: text.to_string(),
                description: None,
                target_date,
                is_recurring,
                recurrence_rule,
                related_person,
                amount,
                currency,
                confidence: Some(0.6),
            };

            if let Ok(json) = serde_json::to_string(&parsed) {
                return CString::new(json).unwrap().into_raw();
            }
        }
        ptr::null_mut()
    }
}

/// FFI: Given a JSON array of ProcessedEvent and a search key (dedupe/id/sourceId),
/// return the best matching ProcessedEvent as JSON string or NULL.
#[no_mangle]
pub extern "C" fn nola_core_find_in_processed_list(
    data_ptr: *const u8,
    data_len: usize,
    key_ptr: *const c_char,
) -> *mut c_char {
    if data_ptr.is_null() || data_len == 0 || key_ptr.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        let key = match CStr::from_ptr(key_ptr).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        };

        if let Ok(text) = std::str::from_utf8(slice) {
            if let Ok(list) = serde_json::from_str::<Vec<ProcessedEvent>>(text) {
                // 1. exact dedupeKey or id
                if let Some(found) = list.iter().find(|e| e.dedupe_key == key || e.id == key) {
                    if let Ok(json) = serde_json::to_string(found) {
                        return CString::new(json).unwrap().into_raw();
                    }
                }
                // 2. partial match
                if let Some(found) = list
                    .iter()
                    .find(|e| e.dedupe_key.contains(&key) || key.contains(&e.dedupe_key))
                {
                    if let Ok(json) = serde_json::to_string(found) {
                        return CString::new(json).unwrap().into_raw();
                    }
                }
                // 3. sourceId match
                if let Some(found) = list.iter().find(|e| e.source_id == key) {
                    if let Ok(json) = serde_json::to_string(found) {
                        return CString::new(json).unwrap().into_raw();
                    }
                }
            }
        }
        ptr::null_mut()
    }
}

/// Generate dedupe key
pub(crate) fn generate_dedupe_key_internal(
    source: &str,
    title: &str,
    date_iso: &str,
    source_id: &str,
) -> String {
    let re = Regex::new(r"\s+").unwrap();
    let normalized = re.replace_all(title.trim(), " ").to_lowercase();
    let digest = md5::compute(format!("{}:{}:{}", source, normalized, date_iso));
    format!("{}:{:x}:{}", source, digest, source_id)
}

pub(crate) fn hash_str_i64(s: &str) -> i64 {
    let digest = md5::compute(s.as_bytes());
    let bytes = digest.0;
    i64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// FFI: Generate dedupe key
#[no_mangle]
pub extern "C" fn nola_core_generate_dedupe_key(
    source: *const c_char,
    title: *const c_char,
    date_iso: *const c_char,
    source_id: *const c_char,
) -> *mut c_char {
    unsafe {
        if source.is_null() || title.is_null() {
            return ptr::null_mut();
        }
        let s_src = CStr::from_ptr(source).to_string_lossy();
        let s_title = CStr::from_ptr(title).to_string_lossy();
        let s_date = if date_iso.is_null() {
            std::borrow::Cow::Borrowed("")
        } else {
            CStr::from_ptr(date_iso).to_string_lossy()
        };
        let s_id = if source_id.is_null() {
            std::borrow::Cow::Borrowed("")
        } else {
            CStr::from_ptr(source_id).to_string_lossy()
        };
        let key = generate_dedupe_key_internal(&s_src, &s_title, &s_date, &s_id);
        CString::new(key).unwrap().into_raw()
    }
}

/// FFI: Parse URL JSON
#[no_mangle]
pub extern "C" fn nola_core_parse_url_json(data_ptr: *const u8, data_len: usize) -> *mut c_char {
    if data_ptr.is_null() || data_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        match parse_url_json_internal(slice) {
            Ok(events) => {
                if let Ok(json) = serde_json::to_string(&events) {
                    return CString::new(json).unwrap().into_raw();
                }
            }
            Err(e) => {
                if let Ok(err_json) = serde_json::to_string(&serde_json::json!({"error": e})) {
                    return CString::new(err_json).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}

/// FFI: Process raw events
#[no_mangle]
pub extern "C" fn nola_core_process_raw_events(
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
                let mut processed = vec![];
                for re in raw_events {
                    let dt = re.date.as_ref().and_then(|d| {
                        DateTime::parse_from_rfc3339(d)
                            .or_else(|_| DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", d)))
                            .ok()
                    });
                    let dedupe_key = generate_dedupe_key_internal(
                        &re.source,
                        &re.title,
                        dt.as_ref().map(|d| d.to_rfc3339()).as_deref().unwrap_or(""),
                        &re.id,
                    );
                    let pe = ProcessedEvent {
                        id: re.id.clone(),
                        source: re.source.clone(),
                        source_type: re.source.clone(),
                        source_id: re.id.clone(),
                        title: re.title.clone(),
                        date: dt.map(|d| d.to_rfc3339()),
                        start: None,
                        due: None,
                        priority: 0.5,
                        tags: None,
                        dedupe_key,
                        provenance: None,
                        metadata: re.metadata.clone(),
                        notes: re.notes.clone(),
                        hash: hash_str_i64(&re.title),
                    };
                    processed.push(pe);
                }
                if let Ok(out) = serde_json::to_string(&processed) {
                    return CString::new(out).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}
