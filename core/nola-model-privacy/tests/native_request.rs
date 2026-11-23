// SPDX-License-Identifier: Apache-2.0
use nola_model_privacy::{nola_core_redact_model_request_json, nola_model_privacy_free};
use std::{ffi::CStr, ptr};

#[test]
fn native_caller_receives_only_filtered_json_and_can_release_it() {
    let input = br#"{"input":"contact qa@example.org or use fixture-credential"}"#;
    let key = b"fixture-credential";
    unsafe {
        let result = nola_core_redact_model_request_json(
            input.as_ptr(),
            input.len(),
            key.as_ptr(),
            key.len(),
        );
        assert!(!result.is_null());
        let wire: serde_json::Value =
            serde_json::from_slice(CStr::from_ptr(result).to_bytes()).unwrap();
        nola_model_privacy_free(result);
        assert_eq!(wire["input"], "contact [EMAIL] or use [CREDENTIAL]");
    }
}

#[test]
fn malformed_inputs_cannot_fall_back_to_the_unfiltered_body() {
    for input in [&b"broken"[..], b"null", b"[]"] {
        unsafe {
            assert!(nola_core_redact_model_request_json(
                input.as_ptr(),
                input.len(),
                ptr::null(),
                0
            )
            .is_null());
        }
    }
    let input = b"{}";
    unsafe {
        assert!(
            nola_core_redact_model_request_json(input.as_ptr(), input.len(), ptr::null(), 1)
                .is_null()
        );
        nola_model_privacy_free(ptr::null_mut());
    }
}
