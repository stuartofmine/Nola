// SPDX-License-Identifier: Apache-2.0
use std::{ffi::CString, os::raw::c_char};

/// Caller invokes on an IO worker and frees with nola_core_free_c_string.
#[no_mangle]
pub unsafe extern "C" fn nola_core_software_update_json(input: *const u8, length: usize) -> *mut c_char {
    if input.is_null() || length > 65536 { return std::ptr::null_mut(); }
    let result = std::panic::catch_unwind(|| {
        let text = std::str::from_utf8(std::slice::from_raw_parts(input, length)).ok()?;
        CString::new(nola_update_host::dispatch(text).to_string()).ok().map(CString::into_raw)
    });
    result.ok().flatten().unwrap_or(std::ptr::null_mut())
}
