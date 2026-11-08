// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements utils logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
// Utilities and FFI helpers

use std::ffi::CString;
use std::os::raw::c_char;

/// Free C string returned by the library
#[no_mangle]
pub extern "C" fn nola_core_free_c_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

/// Transform model output JSON (internal utility passthrough)
#[no_mangle]
pub extern "C" fn nola_core_transform_model_output_passthrough(
    data_ptr: *const u8,
    data_len: usize,
) -> *mut c_char {
    use std::ptr;
    if data_ptr.is_null() || data_len == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        // Simple passthrough for now (can add transformation logic if needed)
        if let Ok(s) = std::str::from_utf8(slice) {
            return CString::new(s).unwrap().into_raw();
        }
        ptr::null_mut()
    }
}
