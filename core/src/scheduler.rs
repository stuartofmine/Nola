// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements scheduler logic in the Rust core module.
// - Interfaces:
//   - ScheduleResult
// - APIs:
//   - none
//
// Event scheduling logic

use crate::types::*;
use chrono::{DateTime, NaiveDate, Utc};
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ScheduleResult {
    now: Vec<ProcessedEvent>,
    later: Vec<ProcessedEvent>,
}

/// FFI: Schedule events
#[no_mangle]
pub extern "C" fn nola_core_scheduler_schedule(
    events_ptr: *const u8,
    events_len: usize,
    target_hour: i32,
) -> *mut c_char {
    if events_ptr.is_null() || events_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(events_ptr, events_len);
        if let Ok(s) = std::str::from_utf8(slice) {
            if let Ok(events) = serde_json::from_str::<Vec<ProcessedEvent>>(s) {
                let now_utc = Utc::now();
                let mut now_list = vec![];
                let mut later_list = vec![];

                for ev in events {
                    if let Some(date_str) = &ev.date {
                        if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                            let dt_utc = dt.with_timezone(&Utc);
                            if dt_utc <= now_utc {
                                now_list.push(ev);
                            } else {
                                later_list.push(ev);
                            }
                            continue;
                        }
                        if let Ok(nd) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                            let nd_time = nd
                                .and_hms_opt(target_hour as u32, 0, 0)
                                .unwrap_or_else(|| nd.and_hms_opt(0, 0, 0).unwrap());
                            let dt_utc = DateTime::<Utc>::from_naive_utc_and_offset(nd_time, Utc);
                            if dt_utc <= now_utc {
                                now_list.push(ev);
                            } else {
                                later_list.push(ev);
                            }
                            continue;
                        }
                    }
                    now_list.push(ev);
                }

                let res = ScheduleResult {
                    now: now_list,
                    later: later_list,
                };
                if let Ok(out) = serde_json::to_string(&res) {
                    return CString::new(out).unwrap().into_raw();
                }
            }
        }
        ptr::null_mut()
    }
}
