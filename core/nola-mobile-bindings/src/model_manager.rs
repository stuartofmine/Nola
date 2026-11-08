// SPDX-License-Identifier: Apache-2.0
//! Thin C ABI for the platform Model Runtime Host.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;

pub use nola_model_runtime_host::ModelManager;

#[no_mangle]
pub extern "C" fn nola_core_model_manager_new() -> *mut ModelManager {
    Box::into_raw(Box::new(ModelManager::new()))
}

#[no_mangle]
pub extern "C" fn nola_core_model_manager_free(pointer: *mut ModelManager) {
    if !pointer.is_null() {
        unsafe { drop(Box::from_raw(pointer)) };
    }
}

#[no_mangle]
pub extern "C" fn nola_core_model_sha256(path_ptr: *const c_char) -> *mut c_char {
    if path_ptr.is_null() {
        return ptr::null_mut();
    }
    let path = unsafe { PathBuf::from(CStr::from_ptr(path_ptr).to_string_lossy().as_ref()) };
    ModelManager::sha256_of_file(&path)
        .ok()
        .and_then(|digest| CString::new(digest).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn nola_core_model_manager_download(
    pointer: *mut ModelManager,
    url_ptr: *const c_char,
    destination_ptr: *const c_char,
) -> bool {
    if pointer.is_null() || url_ptr.is_null() || destination_ptr.is_null() {
        return false;
    }
    let manager = unsafe { &*pointer };
    let url = unsafe { CStr::from_ptr(url_ptr).to_string_lossy().to_string() };
    let destination =
        unsafe { PathBuf::from(CStr::from_ptr(destination_ptr).to_string_lossy().as_ref()) };
    manager.download_file(&url, &destination).is_ok()
}

#[no_mangle]
pub extern "C" fn nola_core_model_manager_progress(
    pointer: *mut ModelManager,
    url_ptr: *const c_char,
) -> f64 {
    if pointer.is_null() || url_ptr.is_null() {
        return 0.0;
    }
    let manager = unsafe { &*pointer };
    let url = unsafe { CStr::from_ptr(url_ptr).to_string_lossy() };
    manager.get_progress(&url).unwrap_or(0.0)
}

#[no_mangle]
pub extern "C" fn nola_core_model_manager_cancel(
    pointer: *mut ModelManager,
    url_ptr: *const c_char,
) -> bool {
    if pointer.is_null() || url_ptr.is_null() {
        return false;
    }
    let manager = unsafe { &*pointer };
    let url = unsafe { CStr::from_ptr(url_ptr).to_string_lossy() };
    manager.cancel_download(&url)
}
