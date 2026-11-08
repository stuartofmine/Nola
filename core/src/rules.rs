// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements rules logic in the Rust core module.
// - Interfaces:
//   - RulesEngineRust
// - APIs:
//   - none
//
// Rules engine for filtering and throttling notifications

use crate::types::*;
use chrono::{Timelike, Utc};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::{LazyLock, RwLock};

pub struct RulesEngineRust {
    config: RulesConfig,
    sent_history: HashMap<String, String>, // dedupeKey -> timestamp
}

static GLOBAL_RULES_ENGINE: LazyLock<RwLock<Option<RulesEngineRust>>> =
    LazyLock::new(|| RwLock::new(None));

impl RulesEngineRust {
    pub fn new(config: RulesConfig) -> Self {
        RulesEngineRust {
            config,
            sent_history: HashMap::new(),
        }
    }

    pub fn filter(&mut self, events: Vec<ProcessedEvent>) -> Vec<NotificationCandidate> {
        let mut result = vec![];
        for ev in events {
            if ev.priority < self.config.priority_threshold {
                continue;
            }
            if self.is_recently_sent(&ev.dedupe_key) {
                continue;
            }
            let cand = NotificationCandidate {
                event: ev.clone(),
                dedupe_key: ev.dedupe_key.clone(),
                text: ev.title.clone(),
                action_type: String::from("openCalendar"),
            };
            result.push(cand);
        }
        result
    }

    pub fn is_quiet_hours(&self) -> bool {
        let now = Utc::now();
        let h = now.hour() as i32;
        let start = self.config.quiet_hours_start;
        let end = self.config.quiet_hours_end;
        if start < end {
            h >= start && h < end
        } else {
            h >= start || h < end
        }
    }

    pub fn mark_sent(&mut self, dedupe_key: &str) {
        self.sent_history
            .insert(dedupe_key.to_string(), Utc::now().to_rfc3339());
    }

    fn is_recently_sent(&self, dedupe_key: &str) -> bool {
        if let Some(ts_str) = self.sent_history.get(dedupe_key) {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                let elapsed = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
                let window = chrono::Duration::hours(self.config.throttle_window_hours as i64);
                return elapsed < window;
            }
        }
        false
    }

    pub fn clear_history(&mut self) {
        self.sent_history.clear();
    }
}

/// FFI: Initialize/replace the global RulesEngine with a JSON config
#[no_mangle]
pub extern "C" fn nola_core_rules_init(config_ptr: *const u8, config_len: usize) -> bool {
    if config_ptr.is_null() || config_len == 0 {
        return false;
    }
    unsafe {
        let slice = std::slice::from_raw_parts(config_ptr, config_len);
        if let Ok(config_str) = std::str::from_utf8(slice) {
            if let Ok(cfg) = serde_json::from_str::<RulesConfig>(config_str) {
                let mut guard = GLOBAL_RULES_ENGINE.write().unwrap();
                *guard = Some(RulesEngineRust::new(cfg));
                return true;
            }
        }
        false
    }
}

/// FFI: Filter events through global RulesEngine
#[no_mangle]
pub extern "C" fn nola_core_rules_filter(events_ptr: *const u8, events_len: usize) -> *mut c_char {
    if events_ptr.is_null() || events_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(events_ptr, events_len);
        if let Ok(json_str) = std::str::from_utf8(slice) {
            if let Ok(evts) = serde_json::from_str::<Vec<ProcessedEvent>>(json_str) {
                let mut guard = GLOBAL_RULES_ENGINE.write().unwrap();
                if guard.is_none() {
                    return ptr::null_mut();
                }
                let engine = guard.as_mut().unwrap();
                let res = engine.filter(evts);
                if let Ok(out) = serde_json::to_string(&res) {
                    return CString::new(out).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}

/// FFI: Mark a dedupeKey as sent
#[no_mangle]
pub extern "C" fn nola_core_rules_mark_sent(dedupe_ptr: *const c_char) {
    if dedupe_ptr.is_null() {
        return;
    }
    unsafe {
        let dedupe = CStr::from_ptr(dedupe_ptr).to_string_lossy().to_string();
        let mut guard = GLOBAL_RULES_ENGINE.write().unwrap();
        if let Some(engine) = guard.as_mut() {
            engine.mark_sent(&dedupe);
        }
    }
}

/// FFI: Check if current time is in quiet hours
#[no_mangle]
pub extern "C" fn nola_core_rules_is_quiet_hours() -> bool {
    let guard = GLOBAL_RULES_ENGINE.read().unwrap();
    if let Some(engine) = guard.as_ref() {
        engine.is_quiet_hours()
    } else {
        false
    }
}

/// FFI: Clear sent history
#[no_mangle]
pub extern "C" fn nola_core_rules_clear_history() {
    let mut guard = GLOBAL_RULES_ENGINE.write().unwrap();
    if let Some(engine) = guard.as_mut() {
        engine.clear_history();
    }
}
