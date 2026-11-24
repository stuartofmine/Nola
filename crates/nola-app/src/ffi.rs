// SPDX-License-Identifier: Apache-2.0
//! Clean C-ABI bridge for macOS Swift (`NolaCore.framework`) and Android JNI consumers.

use std::ffi::{c_char, CStr};

use nola_store::PROTOCOL_VERSION;
use nola_types::estimate_text_tokens;

const APPLICATION_ABI_VERSION: u32 = 1;
static VERSION_CSTR: &[u8] = b"2.0.0\0";

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
    fn ffi_contract_matches_macos_client_expectations() {
        assert_eq!(nola_core_application_abi_version(), 1);
        assert_eq!(nola_core_application_service_protocol_version(), 1);
        let sample = CString::new("你好hello").unwrap();
        assert_eq!(unsafe { nola_v2_estimate_tokens(sample.as_ptr()) }, 4);
    }
}
