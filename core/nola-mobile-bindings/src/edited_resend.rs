// SPDX-License-Identifier: Apache-2.0
//! Embedded trusted-Host-only policy attestation. No ordinary command JSON alias.
use super::{encode, utf8_input, NolaCoreApplication};
use serde_json::json;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FreshHostPayload {
    core_config: nola_core::agent::AgentCoreConfig,
    submission: nola_core::agent::AgentSubmission,
}
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn nola_core_application_attest_resend_host_policy(
    application: *mut NolaCoreApplication,
    target_ptr: *const u8,
    target_len: usize,
    config_ptr: *const u8,
    config_len: usize,
) -> *mut c_char {
    let fail = |message: &str| {
        encode(json!({"ok":false,"error":{"code":"resend_host_policy_rejected","message":message}}))
    };
    if application.is_null() {
        return fail("Nola Application is unavailable");
    }
    if target_len > 512 || config_len > nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES {
        return fail("Host policy input exceeds its bound");
    }
    let Some(target) = utf8_input(target_ptr, target_len) else {
        return fail("Target is not UTF-8");
    };
    let Some(raw) = utf8_input(config_ptr, config_len) else {
        return fail("Host configuration is not UTF-8");
    };
    let payload: FreshHostPayload = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return fail("Invalid fresh Host configuration"),
    };
    let runtime = unsafe { &*application };
    let mut runtime = match runtime.application.lock() {
        Ok(value) => value,
        Err(_) => return fail("Application lock is poisoned"),
    };
    match runtime.execute(
        nola_application::ApplicationCommand::AttestEditedResendHostPolicy {
            conversation_id: target,
            core_config: payload.core_config,
            submission: payload.submission,
        },
    ) {
        Ok(result) => encode(json!({"ok":true,"result":result})),
        Err(_) => fail("The prepared target and fresh Host policy could not be verified"),
    }
}
