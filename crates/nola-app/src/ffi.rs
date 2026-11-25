// SPDX-License-Identifier: Apache-2.0
//! Clean C-ABI bridge for Apple (`NolaCore.framework` / Swift) and Android (JNI) consumers.
//! Canonical header: `crates/nola-app/include/nola_core.h`.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use nola_llm::PrivacyRedactor;
use nola_modules::{check_for_update, ModulesSuite};
use nola_store::{Store, PROTOCOL_VERSION};
use nola_types::estimate_text_tokens;
use serde_json::{json, Value};

use crate::daemon::handle_envelope;

const APPLICATION_ABI_VERSION: u32 = 1;
static VERSION_CSTR: &[u8] = b"2.0.0\0";

thread_local! {
    static LAST_OPEN_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_open_error(err: Option<String>) {
    LAST_OPEN_ERROR.with(|slot| {
        *slot.borrow_mut() = err;
    });
}

/// Opaque handle representing an opened Nola v2 Store + ModulesSuite across C-ABI.
pub struct NolaCoreApplication {
    store: Store,
    suite: ModulesSuite,
}

fn to_owned_c_string(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

fn scrub_credential_in_value(val: &mut Value, credential: &str) {
    match val {
        Value::String(s) => {
            if s.contains(credential) {
                *s = s.replace(credential, "[REDACTED_CREDENTIAL]");
            }
        }
        Value::Array(arr) => {
            for item in arr {
                scrub_credential_in_value(item, credential);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                scrub_credential_in_value(v, credential);
            }
        }
        _ => {}
    }
}

/// Free a C string previously returned by any `nola_core_*` function.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by `nola_core_*`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

/// Alias for `nola_core_free_c_string` used by Apple/Android model privacy bridges.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by `nola_core_redact_model_request_json`.
#[no_mangle]
pub unsafe extern "C" fn nola_model_privacy_free(ptr: *mut c_char) {
    unsafe { nola_core_free_c_string(ptr) };
}

#[no_mangle]
pub extern "C" fn nola_core_application_abi_version() -> u32 {
    APPLICATION_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_application_service_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[no_mangle]
pub extern "C" fn nola_v2_version() -> *const c_char {
    VERSION_CSTR.as_ptr().cast()
}

/// Open a Nola v2 application store handle.
///
/// # Safety
/// `path_utf8` must point to `path_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_open(
    path_utf8: *const u8,
    path_len: usize,
) -> *mut NolaCoreApplication {
    set_last_open_error(None);
    if path_utf8.is_null() || path_len == 0 {
        set_last_open_error(Some("empty database path".into()));
        return std::ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(path_utf8, path_len) };
    let path_str = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(e) => {
            set_last_open_error(Some(format!("invalid UTF-8 database path: {e}")));
            return std::ptr::null_mut();
        }
    };
    let db_path = PathBuf::from(path_str);
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            set_last_open_error(Some(format!("failed to open store: {e}")));
            return std::ptr::null_mut();
        }
    };
    let ws = db_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let suite = ModulesSuite::new(store.clone(), ws, None);
    Box::into_raw(Box::new(NolaCoreApplication { store, suite }))
}

/// Return the last error message from `nola_core_application_open` on the current thread, or NULL.
#[no_mangle]
pub extern "C" fn nola_core_application_last_open_error() -> *mut c_char {
    LAST_OPEN_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(msg) => to_owned_c_string(msg.clone()),
        None => std::ptr::null_mut(),
    })
}

/// Free a handle returned by `nola_core_application_open`.
///
/// # Safety
/// `app` must be null or a valid pointer returned by `nola_core_application_open`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_free(app: *mut NolaCoreApplication) {
    if !app.is_null() {
        drop(unsafe { Box::from_raw(app) });
    }
}

/// Execute a JSON envelope against an opened `NolaCoreApplication` and return an owned JSON C string.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_execute(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() || json_ptr.is_null() {
        return to_owned_c_string(r#"{"status":"rejected","error":{"message":"null pointer"}}"#.into());
    }
    let app_ref = unsafe { &*app };
    let slice = unsafe { std::slice::from_raw_parts(json_ptr, json_len) };
    let Ok(req) = serde_json::from_slice::<Value>(slice) else {
        return to_owned_c_string(r#"{"status":"rejected","error":{"message":"invalid json"}}"#.into());
    };
    let resp = handle_envelope(&app_ref.store, &app_ref.suite, &req);
    to_owned_c_string(resp.to_string())
}

/// Submit an idempotent command envelope (alias of `nola_core_application_execute` for mobile runtimes).
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_submit(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    unsafe { nola_core_application_execute(app, json_ptr, json_len) }
}

/// Redact high-confidence secrets/tokens (and an optional active credential) in a JSON payload
/// before cloud egress without breaking JSON structure.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes; if `credential_len > 0`, `credential` must
/// point to `credential_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_redact_model_request_json(
    input: *const u8,
    size: usize,
    credential: *const u8,
    credential_len: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 {
        return to_owned_c_string(String::new());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let cred_str = if !credential.is_null() && credential_len > 0 {
        let cred_slice = unsafe { std::slice::from_raw_parts(credential, credential_len) };
        let trimmed = String::from_utf8_lossy(cred_slice).trim().to_string();
        (trimmed.len() >= 6).then_some(trimmed)
    } else {
        None
    };

    if let Ok(mut json_val) = serde_json::from_slice::<Value>(slice) {
        PrivacyRedactor::redact_json_value(&mut json_val);
        if let Some(ref cred) = cred_str {
            scrub_credential_in_value(&mut json_val, cred);
        }
        return to_owned_c_string(json_val.to_string());
    }

    let text = String::from_utf8_lossy(slice);
    let mut out = PrivacyRedactor::redact_text(&text);
    if let Some(ref cred) = cred_str {
        out = out.replace(cred, "[REDACTED_CREDENTIAL]");
    }
    to_owned_c_string(out)
}

/// Check a signed software update feed manifest from JSON request:
/// `{"manifest_url":"https://...","current_version":"2.0.0"}`.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_software_update_json(
    input: *const u8,
    size: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 {
        return to_owned_c_string(json!({"ok": false, "error": "empty request"}).to_string());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let req: Value = match serde_json::from_slice(slice) {
        Ok(v) => v,
        Err(e) => {
            return to_owned_c_string(json!({"ok": false, "error": format!("invalid json: {e}")}).to_string());
        }
    };
    let manifest_url = req
        .get("manifest_url")
        .or_else(|| req.get("feed_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let current_version = req
        .get("current_version")
        .and_then(|v| v.as_str())
        .unwrap_or("2.0.0");
    if manifest_url.is_empty() {
        return to_owned_c_string(json!({"ok": false, "error": "missing manifest_url"}).to_string());
    }
    match check_for_update(manifest_url, current_version) {
        Ok(update) => to_owned_c_string(json!({"ok": true, "update": update}).to_string()),
        Err(err) => to_owned_c_string(json!({"ok": false, "error": err}).to_string()),
    }
}

/// Safe UTF-8 token estimator exported across the C-ABI boundary.
///
/// # Safety
/// `text` must be either null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn nola_v2_estimate_tokens(text: *const c_char) -> usize {
    if text.is_null() {
        return 0;
    }
    match unsafe { CStr::from_ptr(text) }.to_str() {
        Ok(s) => estimate_text_tokens(s),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn ffi_open_execute_redact_and_free_roundtrip() {
        assert_eq!(nola_core_application_abi_version(), 1);
        assert_eq!(nola_core_application_service_protocol_version(), 1);
        let sample = CString::new("你好hello").unwrap();
        assert_eq!(unsafe { nola_v2_estimate_tokens(sample.as_ptr()) }, 4);

        // Open error reporting
        let bad = unsafe { nola_core_application_open(std::ptr::null(), 0) };
        assert!(bad.is_null());
        let err_ptr = nola_core_application_last_open_error();
        assert!(!err_ptr.is_null());
        let err_str = unsafe { CStr::from_ptr(err_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(err_ptr) };
        assert!(err_str.contains("empty database path"));

        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("ffi.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());
        assert!(nola_core_application_last_open_error().is_null());

        let cmd = r#"{"request_id":"r1","client_id":"c1","command":{"type":"health","arguments":{}}}"#;
        let out_ptr = unsafe { nola_core_application_submit(app, cmd.as_ptr(), cmd.len()) };
        let out_str = unsafe { CStr::from_ptr(out_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(out_ptr) };
        assert!(out_str.contains("\"status\":\"committed\""));

        let payload = r#"{"prompt":"key=sk-1234567890abcdefghijklmnop token=custom-secret-999"}"#;
        let cred = "custom-secret-999";
        let red_ptr = unsafe {
            nola_core_redact_model_request_json(
                payload.as_ptr(),
                payload.len(),
                cred.as_ptr(),
                cred.len(),
            )
        };
        let red_str = unsafe { CStr::from_ptr(red_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_model_privacy_free(red_ptr) };
        let parsed: Value = serde_json::from_str(&red_str).unwrap();
        assert_eq!(
            parsed["prompt"],
            "key=[REDACTED_API_KEY] token=[REDACTED_CREDENTIAL]"
        );

        let upd_req = r#"{"manifest_url":"http://insecure.example.com/feed.json"}"#;
        let upd_ptr = unsafe { nola_core_software_update_json(upd_req.as_ptr(), upd_req.len()) };
        let upd_str = unsafe { CStr::from_ptr(upd_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(upd_ptr) };
        assert!(upd_str.contains("\"ok\":false"));

        unsafe { nola_core_application_free(app) };
    }
}
