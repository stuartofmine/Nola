// SPDX-License-Identifier: Apache-2.0
//! Native ABI adapter; all business computation is owned by Agent engine.
pub use nola_agent_engine::memory::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
#[no_mangle]
pub extern "C" fn nola_core_memory_selection_abi_version() -> u32 {
    MEMORY_SELECTION_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_memory_select_json(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(input) = serde_json::from_slice::<MemorySelectionInput>(bytes) else {
        return ptr::null_mut();
    };
    serde_json::to_string(&select_memories(input))
        .ok()
        .and_then(|json| CString::new(json).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}
