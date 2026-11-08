// SPDX-License-Identifier: Apache-2.0
//! Native ABI adapter; all business computation is owned by Agent engine.
pub use nola_agent_engine::tool_policy::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

#[no_mangle]
pub extern "C" fn nola_core_tool_policy_mcp_allowed(input_ptr: *const u8, input_len: usize) -> bool {
    #[derive(serde::Deserialize)]
    struct Input { policy: ToolPolicyInput, descriptor: crate::agent::host::HostCapabilityDescriptor }
    if input_ptr.is_null() || input_len == 0 { return false; }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    serde_json::from_slice::<Input>(bytes).ok()
        .is_some_and(|input| mcp_host_descriptor_allowed(&input.policy, &input.descriptor))
}
#[no_mangle]
pub extern "C" fn nola_core_tool_policy_abi_version() -> u32 {
    TOOL_POLICY_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_tool_policy_resolve(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let input = match serde_json::from_slice::<ToolPolicyInput>(bytes) {
        Ok(value) => value,
        Err(_) => return ptr::null_mut(),
    };
    let output = match serde_json::to_string(&resolve_tool_policy(&input)) {
        Ok(value) => value,
        Err(_) => return ptr::null_mut(),
    };
    CString::new(output)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}
