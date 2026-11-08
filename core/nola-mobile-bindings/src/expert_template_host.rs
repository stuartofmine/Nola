// SPDX-License-Identifier: Apache-2.0
//! Native Host factory transport. This operation is never forwarded from a
//! renderer's generic Application command or model tool arguments.
use super::{encode, utf8_input, NolaCoreApplication};
use nola_application::host_expert_template_operation::HostExpertTemplateRequest;
use serde_json::json;
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn nola_core_application_expert_host_operation(
    application: *mut NolaCoreApplication,
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    let fail = |message: &str| {
        encode(
            json!({"ok":false,"error":{"code":"expert_host_operation_rejected","message":message}}),
        )
    };
    if application.is_null() {
        return fail("Nola Application is unavailable");
    }
    if input_len > nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES {
        return fail("Expert Host input exceeds its bound");
    }
    let Some(raw) = utf8_input(input_ptr, input_len) else {
        return fail("Expert Host operation is not UTF-8");
    };
    let request: HostExpertTemplateRequest = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return fail("Invalid current Expert Host operation"),
    };
    let application = unsafe { &*application };
    let mut runtime = match application.application.lock() {
        Ok(value) => value,
        Err(_) => {
            if matches!(&request.operation,
                nola_application::host_expert_template_operation::HostExpertTemplateOperation::Admit { .. } |
                nola_application::host_expert_template_operation::HostExpertTemplateOperation::PrepareEditedResend { .. }) {
                return encode(json!({"ok":false,"error":{"code":"expert_host_outcome_pending",
                    "message":"The exact original intent requires owner reconciliation","outcome_pending":true,
                    "reopen_required":true,"request_id":request.request_id}}));
            }
            return fail("Application lock is poisoned");
        }
    };
    let request_id = request.request_id;
    match runtime.execute_host_expert_template_operation("native-expert-host", &request_id, request.operation) {
        Ok(result) => encode(json!({"ok":true,"result":result,"request_id":request_id})),
        Err(nola_application::host_expert_template_operation::HostExpertOperationError::Rejected { code, message }) => {
            encode(json!({"ok":false,"error":{"code":code,"message":message,"outcome_pending":false,"request_id":request_id}}))
        }
        Err(nola_application::host_expert_template_operation::HostExpertOperationError::OutcomePending { request_id, message, reopen_required }) => {
            if reopen_required {
                // Reopen the actual shared owner, not just this thin handle.
                // The original receipt remains uncertain until exact replay.
                let _ = runtime.reopen();
            }
            encode(json!({"ok":false,"error":{"code":"expert_host_outcome_pending","message":message,"outcome_pending":true,"request_id":request_id,"reopen_required":reopen_required}}))
        }
    }
}
