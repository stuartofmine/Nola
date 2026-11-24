// SPDX-License-Identifier: Apache-2.0
//! Clean C-ABI bridge for macOS Swift (`NolaCore.framework`) and Android JNI consumers.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use nola_llm::PrivacyRedactor;
use nola_modules::ModulesSuite;
use nola_store::{Store, PROTOCOL_VERSION};
use nola_types::estimate_text_tokens;
use serde_json::Value;

use crate::daemon::handle_envelope;

const APPLICATION_ABI_VERSION: u32 = 1;
static VERSION_CSTR: &[u8] = b"2.0.0\0";

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
    if path_utf8.is_null() || path_len == 0 {
        return std::ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(path_utf8, path_len) };
    let Ok(path_str) = std::str::from_utf8(slice) else {
        return std::ptr::null_mut();
    };
    let db_path = PathBuf::from(path_str);
    let Ok(store) = Store::open(&db_path) else {
        return std::ptr::null_mut();
    };
    let ws = db_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let suite = ModulesSuite::new(store.clone(), ws, None);
    Box::into_raw(Box::new(NolaCoreApplication { store, suite }))
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

/// Redact high-confidence secrets/tokens in a JSON payload before cloud egress.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_redact_model_request_json(
    input: *const u8,
    size: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 {
        return to_owned_c_string(String::new());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let text = String::from_utf8_lossy(slice);
    to_owned_c_string(PrivacyRedactor::redact_text(&text))
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

        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("ffi.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());

        let cmd = r#"{"request_id":"r1","client_id":"c1","command":{"type":"health","arguments":{}}}"#;
        let out_ptr = unsafe { nola_core_application_execute(app, cmd.as_ptr(), cmd.len()) };
        let out_str = unsafe { CStr::from_ptr(out_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(out_ptr) };
        assert!(out_str.contains("\"status\":\"committed\""));

        let secret = "api=sk-1234567890abcdefghijklmnop";
        let red_ptr = unsafe { nola_core_redact_model_request_json(secret.as_ptr(), secret.len()) };
        let red_str = unsafe { CStr::from_ptr(red_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(red_ptr) };
        assert_eq!(red_str, "api=[REDACTED_API_KEY]");

        unsafe { nola_core_application_free(app) };
    }
}
