// SPDX-License-Identifier: Apache-2.0
//! Native ABI adapter; all business computation is owned by Agent engine.
pub use nola_agent_engine::context_engine::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use serde::Serialize;
fn input_bytes<'a>(input_ptr: *const u8, input_len: usize) -> Option<&'a [u8]> {
    if input_ptr.is_null() || input_len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(input_ptr, input_len) })
}

fn json_c_string<T: Serialize>(value: &T) -> *mut c_char {
    serde_json::to_string(value)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn nola_core_context_plan(input_ptr: *const u8, input_len: usize) -> *mut c_char {
    let Some(bytes) = input_bytes(input_ptr, input_len) else {
        return ptr::null_mut();
    };
    let Ok(input) = serde_json::from_slice::<ContextPlanInput>(bytes) else {
        return ptr::null_mut();
    };
    json_c_string(&plan_context(&input))
}

#[no_mangle]
pub extern "C" fn nola_core_context_engine_abi_version() -> u32 {
    CONTEXT_ENGINE_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_context_prepare(input_ptr: *const u8, input_len: usize) -> *mut c_char {
    let Some(bytes) = input_bytes(input_ptr, input_len) else {
        return ptr::null_mut();
    };
    let Ok(input) = serde_json::from_slice::<ContextPrepareInput>(bytes) else {
        return ptr::null_mut();
    };
    json_c_string(&prepare_context(&input))
}
