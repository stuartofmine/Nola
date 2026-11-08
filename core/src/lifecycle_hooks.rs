// SPDX-License-Identifier: Apache-2.0
//! Native ABI adapter; all business computation is owned by Agent engine.
pub use nola_agent_engine::lifecycle_hooks::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
#[no_mangle]
pub extern "C" fn nola_core_lifecycle_hook_abi_version() -> u32 {
    LIFECYCLE_HOOK_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_lifecycle_hook_build_registry(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(input) = serde_json::from_slice::<LifecycleHookRegistryInput>(bytes) else {
        return ptr::null_mut();
    };
    let Ok(snapshot) = build_lifecycle_hook_registry(&input) else {
        return ptr::null_mut();
    };
    serde_json::to_string(&snapshot)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn nola_core_lifecycle_hook_plan(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(input) = serde_json::from_slice::<LifecycleHookPlanInput>(bytes) else {
        return ptr::null_mut();
    };
    let Ok(plan) = plan_lifecycle_hooks(&input) else {
        return ptr::null_mut();
    };
    serde_json::to_string(&plan)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn nola_core_lifecycle_hook_project_execution(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(input) = serde_json::from_slice::<LifecycleHookExecutionInput>(bytes) else {
        return ptr::null_mut();
    };
    let Ok(projection) = project_lifecycle_hook_execution(&input) else {
        return ptr::null_mut();
    };
    serde_json::to_string(&projection)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}
