// SPDX-License-Identifier: Apache-2.0
//! Stateful native presenter boundary. No Store, Host, or process ownership.

use nola_domain::{ConversationProjection, ConversationProjectionEvent};
use nola_kernel::{ConversationProjectionReducer, ProjectionApplyOutcome};
use serde_json::json;
use std::{io, os::raw::c_char, ptr, sync::Mutex};

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;

pub struct NolaCoreProjection {
    reducer: Mutex<ConversationProjectionReducer>,
}

struct ByteBudget(usize);

impl io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "projection byte budget exceeded",
            ));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fits_budget(projection: &ConversationProjection) -> bool {
    serde_json::to_writer(ByteBudget(MAX_JSON_BYTES), projection).is_ok()
}

fn input(pointer: *const u8, length: usize) -> Option<String> {
    if length > MAX_JSON_BYTES {
        return None;
    }
    super::utf8_input(pointer, length)
}

fn error(code: &str, message: impl ToString) -> *mut c_char {
    super::encode(json!({"ok": false, "error": {"code": code, "message": message.to_string()}}))
}

/// # Safety
/// Input must be readable for `length` bytes. Returns null on invalid input.
#[no_mangle]
pub unsafe extern "C" fn nola_core_projection_new(
    pointer: *const u8,
    length: usize,
) -> *mut NolaCoreProjection {
    let Some(input) = input(pointer, length) else {
        return ptr::null_mut();
    };
    let Ok(initial) =
        serde_json::from_str::<nola_application::ConversationProjectionWindowV1>(&input)
    else {
        return ptr::null_mut();
    };
    if !fits_budget(&initial.projection) {
        return ptr::null_mut();
    }
    let Some(owners) = initial.stream_owners else {
        return ptr::null_mut();
    };
    let Ok(reducer) = ConversationProjectionReducer::restore_owned(
        initial.projection,
        initial.event_sequence,
        owners,
    ) else {
        return ptr::null_mut();
    };
    Box::into_raw(Box::new(NolaCoreProjection {
        reducer: Mutex::new(reducer),
    }))
}

/// # Safety
/// Handle must be live; input readable for `length` bytes. Free cannot race this call.
#[no_mangle]
pub unsafe extern "C" fn nola_core_projection_apply(
    handle: *mut NolaCoreProjection,
    pointer: *const u8,
    length: usize,
) -> *mut c_char {
    let Some(handle) = handle.as_ref() else {
        return error("invalid_handle", "projection is unavailable");
    };
    let Some(input) = input(pointer, length) else {
        return error("invalid_input", "invalid UTF-8 or input exceeds byte limit");
    };
    let events: Vec<ConversationProjectionEvent> = match serde_json::from_str(&input) {
        Ok(events) => events,
        Err(reason) => return error("invalid_events", reason),
    };
    if events.len() > ConversationProjectionReducer::MAX_BATCH_EVENTS {
        return error("batch_too_large", "maximum 256 events per batch");
    }
    let Ok(mut reducer) = handle.reducer.lock() else {
        return error("poisoned_handle", "projection lock is poisoned");
    };
    let mut staged = reducer.clone();
    let mut counts = [0usize; 3];
    for event in events {
        let index = match staged.apply(event) {
            Ok(ProjectionApplyOutcome::Applied) => 0,
            Ok(ProjectionApplyOutcome::Duplicate) => 1,
            Ok(ProjectionApplyOutcome::Stale) => 2,
            Err(reason) => return error("rejected_event", reason),
        };
        counts[index] += 1;
    }
    // Count encoded bytes without allocating another full snapshot string.
    // This bounded scan still needs performance profiling before UI adoption.
    if !fits_budget(staged.projection()) {
        return error(
            "projection_too_large",
            "retained projection exceeds byte limit",
        );
    }
    *reducer = staged;
    super::encode(json!({"ok": true, "result": {
        "sequence": reducer.last_sequence(), "applied": counts[0],
        "duplicates": counts[1], "stale": counts[2],
    }}))
}

/// # Safety
/// Handle must be live. Free cannot race this call.
#[no_mangle]
pub unsafe extern "C" fn nola_core_projection_snapshot(
    handle: *mut NolaCoreProjection,
) -> *mut c_char {
    let Some(handle) = handle.as_ref() else {
        return error("invalid_handle", "projection is unavailable");
    };
    let Ok(reducer) = handle.reducer.lock() else {
        return error("poisoned_handle", "projection lock is poisoned");
    };
    super::encode(json!({"ok": true, "result": {
        "projection": reducer.projection(), "sequence": reducer.last_sequence(),
    }}))
}

/// # Safety
/// Handle must be null or an unfreed handle returned by new, with no concurrent calls.
#[no_mangle]
pub unsafe extern "C" fn nola_core_projection_free(handle: *mut NolaCoreProjection) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    unsafe fn take(pointer: *mut c_char) -> serde_json::Value {
        assert!(!pointer.is_null());
        serde_json::from_str(CString::from_raw(pointer).to_str().unwrap()).unwrap()
    }

    #[test]
    fn historical_projection_roundtrips_without_live_stream_ownership() {
        let projection: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/historical_conversation_projection_v1.json"
        ))
        .unwrap();
        let window = json!({"event_sequence":0,"total_message_count":1,"before_ordinal":null,
            "has_more":false,"assistant_journal_heads":[],"deferred_text_parts":[],"stream_owners":[],"projection":projection});
        let input = window.to_string();
        unsafe {
            let handle = nola_core_projection_new(input.as_ptr(), input.len());
            assert!(!handle.is_null());
            assert_eq!(
                take(nola_core_projection_snapshot(handle))["result"]["projection"],
                projection
            );
            // A historical text field is not a live Text Part and cannot accept deltas.
            let event = json!([{"identity":{"event_id":"cannot-execute-history","conversation_id":"history",
                "run_id":"run","attempt_id":"attempt","sequence":1},"mutation":{
                "type":"append_text_preview","message_id":"message","part_id":"text","revision":2,"delta":"changed"}}]).to_string();
            assert_eq!(
                take(nola_core_projection_apply(
                    handle,
                    event.as_ptr(),
                    event.len()
                ))["ok"],
                false
            );
            let snapshot = take(nola_core_projection_snapshot(handle));
            assert_eq!(snapshot["result"]["sequence"], 0);
            assert_eq!(snapshot["result"]["projection"], projection);
            nola_core_projection_free(handle);
        }
    }

    #[test]
    fn ffi_accepts_the_actual_application_window_without_reencoding_its_shape() {
        use nola_application::ApplicationCommand;
        use nola_store::{CreateConversationInput, NolaStore};
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("native-window.db");
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "window".into(),
                title: "Application window".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        drop(store);
        let mut runtime = nola_application_bootstrap::open(&database).unwrap();
        let window = runtime
            .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
                conversation_id: "window".into(),
                before_ordinal: None,
                limit: 64,
                content_byte_limit: 512_000,
            })
            .unwrap();
        let input = serde_json::to_vec(&window).unwrap();
        unsafe {
            let handle = nola_core_projection_new(input.as_ptr(), input.len());
            assert!(!handle.is_null());
            let result = take(nola_core_projection_snapshot(handle));
            assert_eq!(result["result"]["projection"], window["projection"]);
            assert_eq!(result["result"]["sequence"], window["event_sequence"]);
            nola_core_projection_free(handle);
        }
    }

    #[test]
    fn ffi_handle_preserves_state_and_rolls_back_rejected_batch() {
        let initial = json!({"event_sequence": 0, "total_message_count": 0, "before_ordinal": null,
            "has_more": false, "assistant_journal_heads": [], "deferred_text_parts": [], "stream_owners": [], "projection": {
            "execution_ownership": {"kind": "user"}, "contract_version": 2, "conversation_id": "native", "revision": 0,
            "title": "Initial", "messages": [], "active_run": null,
            "context_usage": {"used_tokens": 0, "context_window_tokens": 65536,
                "prompt_token_limit": 61440, "estimated": true, "measured_at_revision": 0}
        }})
        .to_string();
        unsafe {
            let handle = nola_core_projection_new(initial.as_ptr(), initial.len());
            assert!(!handle.is_null());
            let event = |sequence| {
                json!({"identity": {
                "event_id": format!("native-{sequence}"), "conversation_id": "native",
                "run_id": null, "attempt_id": null, "sequence": sequence
            }, "mutation": {"type": "set_title", "title": "Updated", "revision": 1}})
            };
            let broken = json!([event(1), event(3)]).to_string();
            assert_eq!(
                take(nola_core_projection_apply(
                    handle,
                    broken.as_ptr(),
                    broken.len()
                ))["ok"],
                false
            );
            let snapshot = take(nola_core_projection_snapshot(handle));
            assert_eq!(snapshot["result"]["sequence"], 0);
            assert_eq!(snapshot["result"]["projection"]["title"], "Initial");
            let valid = json!([event(1)]).to_string();
            let result = take(nola_core_projection_apply(
                handle,
                valid.as_ptr(),
                valid.len(),
            ));
            assert_eq!(result["result"]["applied"], 1);
            assert!(result["result"].get("projection").is_none());
            assert_eq!(
                take(nola_core_projection_apply(
                    handle,
                    valid.as_ptr(),
                    valid.len()
                ))["result"]["duplicates"],
                1
            );
            nola_core_projection_free(handle);
            assert!(nola_core_projection_new(ptr::null(), MAX_JSON_BYTES + 1).is_null());
            assert_eq!(
                take(nola_core_projection_snapshot(ptr::null_mut()))["ok"],
                false
            );
        }
    }

    #[test]
    fn ffi_restore_requires_live_part_ownership_and_rejects_another_attempt() {
        let mut initial = json!({"event_sequence": 5, "total_message_count": 1, "before_ordinal": null,
            "has_more": false, "assistant_journal_heads": [], "deferred_text_parts": [], "stream_owners": [], "projection": {
            "execution_ownership": {"kind": "user"}, "contract_version": 2, "conversation_id": "native", "revision": 1,
            "title": "Live", "messages": [{"message_id": "assistant", "role": "assistant",
                "state": "running", "revision": 1, "created_at_ms": 1, "parts": [{
                    "kind": "text", "part_id": "text", "revision": 1, "markdown": "Hello", "is_final": false
                }]}], "active_run": null,
            "context_usage": {"used_tokens": 0, "context_window_tokens": 65536,
                "prompt_token_limit": 61440, "estimated": true, "measured_at_revision": 0}
        }});
        unsafe {
            let missing = initial.to_string();
            assert!(nola_core_projection_new(missing.as_ptr(), missing.len()).is_null());
            initial["stream_owners"] = json!([{"message_id": "assistant", "part_id": "text",
                "run_id": "run", "execution": {"type": "model_attempt", "attempt_id": "original"}}]);
            let owned = initial.to_string();
            let handle = nola_core_projection_new(owned.as_ptr(), owned.len());
            assert!(!handle.is_null());
            let event = |attempt| {
                json!([{"identity": {"event_id": "continued", "conversation_id": "native",
                "run_id": "run", "attempt_id": attempt, "sequence": 6}, "mutation": {
                "type": "append_text_preview", "message_id": "assistant", "part_id": "text",
                "revision": 2, "delta": " world"}}])
                .to_string()
            };
            let rejected = event("intruder");
            assert_eq!(
                take(nola_core_projection_apply(
                    handle,
                    rejected.as_ptr(),
                    rejected.len()
                ))["ok"],
                false
            );
            assert_eq!(
                take(nola_core_projection_snapshot(handle))["result"]["sequence"],
                5
            );
            let accepted = event("original");
            assert_eq!(
                take(nola_core_projection_apply(
                    handle,
                    accepted.as_ptr(),
                    accepted.len()
                ))["ok"],
                true
            );
            assert_eq!(
                take(nola_core_projection_snapshot(handle))["result"]["projection"]["messages"][0]
                    ["parts"][0]["markdown"],
                "Hello world"
            );
            nola_core_projection_free(handle);
        }
    }
}
