// SPDX-License-Identifier: Apache-2.0
use std::{
    ffi::{CStr, CString},
    os::raw::c_char,
    path::Path,
};

/// Call off the UI thread. Both inputs are NUL-terminated UTF-8 strings.
/// Release the owned result using nola_core_free_c_string.
#[no_mangle]
pub unsafe extern "C" fn nola_mobile_github_skill_package_json(
    root: *const c_char,
    request: *const c_char,
) -> *mut c_char {
    if root.is_null() || request.is_null() {
        return std::ptr::null_mut();
    }
    std::panic::catch_unwind(|| {
        let root = CStr::from_ptr(root).to_str().ok()?;
        let request = CStr::from_ptr(request).to_str().ok()?;
        CString::new(nola_skill_host::mobile_request(Path::new(root), request))
            .ok()
            .map(CString::into_raw)
    })
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}
