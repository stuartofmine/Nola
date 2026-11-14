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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn free_c_string_handles_null_and_allocated_strings() {
        nola_core_free_c_string(ptr::null_mut());
        let raw = CString::new("nola-ffi-string").unwrap().into_raw();
        assert!(!raw.is_null());
        nola_core_free_c_string(raw);
    }
}
