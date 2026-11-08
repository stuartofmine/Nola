// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Platform-neutral data-source policy, validation, and provenance contracts.
// - Interfaces: Native Rust API plus a versioned JSON C ABI for Swift/Kotlin hosts.
// - Security: Validation samples are explicitly redacted and Core never performs source I/O.

use chrono::{DateTime, Utc};
#[cfg(test)]
use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

pub const DATA_SOURCE_ABI_VERSION: u32 = 3;

pub use nola_domain::data_source::{DataSourceReadMethod, DataSourceReadRecipe, DataSourceAccessPolicy, DataSourceValidationState, DataSourceContract, DATA_SOURCE_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDataSourceOverride {
    pub conversation_id: String,
    pub source_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceUsageRecord {
    pub id: String,
    pub source_id: String,
    pub source_title: String,
    pub consumer: String,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub used_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclosed_to_cloud: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DataSourceResolutionInput {
    pub schema_version: u32,
    pub source_id: String,
    pub policy: DataSourceAccessPolicy,
    /// `None` means the conversation has never made an explicit choice.
    pub conversation_override: Option<bool>,
    pub contract: Option<DataSourceContract>,
    pub now: Option<DateTime<Utc>>,
}

impl Default for DataSourceResolutionInput {
    fn default() -> Self {
        Self {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            source_id: String::new(),
            policy: DataSourceAccessPolicy::DefaultEnabled,
            conversation_override: None,
            contract: None,
            now: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceResolutionReason {
    Ready,
    DisabledForConversation,
    ConversationOptInRequired,
    ConsentRequired,
    Blocked,
    ContractMissing,
    ContractInvalid,
    ValidationFailed,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceResolution {
    pub schema_version: u32,
    /// Policy permits the source for this conversation.
    pub enabled: bool,
    /// The source can be injected without another read or permission decision.
    pub usable: bool,
    pub should_request_consent: bool,
    pub should_refresh: bool,
    pub reason: DataSourceResolutionReason,
}

pub fn resolve_data_source(input: &DataSourceResolutionInput) -> DataSourceResolution {
    let policy_resolution = match input.policy {
        DataSourceAccessPolicy::Blocked => {
            Some((false, false, DataSourceResolutionReason::Blocked))
        }
        DataSourceAccessPolicy::DefaultEnabled => input.conversation_override.and_then(|enabled| {
            (!enabled).then_some((
                false,
                false,
                DataSourceResolutionReason::DisabledForConversation,
            ))
        }),
        DataSourceAccessPolicy::ConversationOptIn => match input.conversation_override {
            Some(true) => None,
            Some(false) => Some((
                false,
                false,
                DataSourceResolutionReason::DisabledForConversation,
            )),
            None => Some((
                false,
                false,
                DataSourceResolutionReason::ConversationOptInRequired,
            )),
        },
        DataSourceAccessPolicy::AskWhenRelevant => match input.conversation_override {
            Some(true) => None,
            Some(false) => Some((
                false,
                false,
                DataSourceResolutionReason::DisabledForConversation,
            )),
            None => Some((false, true, DataSourceResolutionReason::ConsentRequired)),
        },
    };

    if let Some((enabled, should_request_consent, reason)) = policy_resolution {
        return DataSourceResolution {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            enabled,
            usable: false,
            should_request_consent,
            should_refresh: false,
            reason,
        };
    }

    let Some(contract) = input.contract.as_ref() else {
        return unresolved(true, true, DataSourceResolutionReason::ContractMissing);
    };
    if contract.validation_state == DataSourceValidationState::Failed {
        return unresolved(true, true, DataSourceResolutionReason::ValidationFailed);
    }
    if !contract.is_structurally_usable(&input.source_id) {
        return unresolved(true, true, DataSourceResolutionReason::ContractInvalid);
    }
    if !contract.is_fresh_at(input.now.unwrap_or_else(Utc::now)) {
        return unresolved(true, true, DataSourceResolutionReason::Stale);
    }

    DataSourceResolution {
        schema_version: DATA_SOURCE_SCHEMA_VERSION,
        enabled: true,
        usable: true,
        should_request_consent: false,
        should_refresh: false,
        reason: DataSourceResolutionReason::Ready,
    }
}

fn unresolved(
    enabled: bool,
    should_refresh: bool,
    reason: DataSourceResolutionReason,
) -> DataSourceResolution {
    DataSourceResolution {
        schema_version: DATA_SOURCE_SCHEMA_VERSION,
        enabled,
        usable: false,
        should_request_consent: false,
        should_refresh,
        reason,
    }
}

#[no_mangle]
pub extern "C" fn nola_core_data_source_abi_version() -> u32 {
    DATA_SOURCE_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_data_source_resolve(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(input) = serde_json::from_slice::<DataSourceResolutionInput>(bytes) else {
        return ptr::null_mut();
    };
    serde_json::to_string(&resolve_data_source(&input))
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Seals a Host-produced validation contract with Core's canonical digest.
/// The Host still owns source I/O, redaction, and persistence; this entry point
/// prevents each client from reimplementing the digest material differently.
#[no_mangle]
pub extern "C" fn nola_core_data_source_seal_contract(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    if input_ptr.is_null() || input_len == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let Ok(mut contract) = serde_json::from_slice::<DataSourceContract>(bytes) else {
        return ptr::null_mut();
    };
    contract.seal_validation();
    serde_json::to_string(&contract)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

#[cfg(test)]
mod seal_contract_tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn ffi_seals_the_same_contract_consumed_by_resolution() {
        let now = Utc::now();
        let contract = DataSourceContract {
            source_id: "plain-text".to_string(),
            connector_id: "test.text".to_string(),
            connector_version: 1,
            reader_description: "Read bounded text".to_string(),
            read_recipe: DataSourceReadRecipe {
                method: DataSourceReadMethod::HostTool,
                operation_id: "test.text.read".to_string(),
                configuration_keys: vec!["sourceID".to_string()],
                output_schema: serde_json::json!({"type": "string"}),
            },
            schema_fields: Vec::new(),
            redacted_sample: "sample".to_string(),
            record_count: 1,
            maximum_age_minutes: 60,
            validation_state: DataSourceValidationState::Verified,
            validated_at: now,
            observed_at: Some(now),
            validation_digest: String::new(),
        };
        let input = serde_json::to_vec(&contract).unwrap();
        let pointer = nola_core_data_source_seal_contract(input.as_ptr(), input.len());
        assert!(!pointer.is_null());
        let raw = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        crate::nola_core_free_c_string(pointer);
        let sealed: DataSourceContract = serde_json::from_str(&raw).unwrap();
        assert!(!sealed.validation_digest.is_empty());
        assert!(sealed.is_structurally_usable("plain-text"));

        let resolution = resolve_data_source(&DataSourceResolutionInput {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            source_id: "plain-text".to_string(),
            policy: DataSourceAccessPolicy::DefaultEnabled,
            conversation_override: None,
            contract: Some(sealed),
            now: Some(now),
        });
        assert_eq!(resolution.reason, DataSourceResolutionReason::Ready);
        assert!(resolution.usable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn contract(now: DateTime<Utc>) -> DataSourceContract {
        let mut contract = DataSourceContract {
            source_id: "calendar".to_string(),
            connector_id: "ios.eventkit".to_string(),
            connector_version: 1,
            reader_description: "Read upcoming calendar events".to_string(),
            read_recipe: DataSourceReadRecipe {
                method: DataSourceReadMethod::PlatformApi,
                operation_id: "ios.eventkit.read-events".to_string(),
                configuration_keys: vec!["calendarIds".to_string(), "dateRange".to_string()],
                output_schema: serde_json::json!({
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string"},
                            "startsAt": {"type": "string"}
                        }
                    }
                }),
            },
            schema_fields: vec!["title".to_string(), "startsAt".to_string()],
            redacted_sample: "Tomorrow, 09:00".to_string(),
            record_count: 2,
            maximum_age_minutes: 60,
            validation_state: DataSourceValidationState::Verified,
            validated_at: now,
            observed_at: Some(now),
            validation_digest: String::new(),
        };
        contract.seal_validation();
        contract
    }

    #[test]
    fn policy_names_match_platform_contract() {
        assert_eq!(
            serde_json::to_string(&DataSourceAccessPolicy::DefaultEnabled).unwrap(),
            "\"defaultEnabled\""
        );
        assert_eq!(
            serde_json::to_string(&DataSourceAccessPolicy::ConversationOptIn).unwrap(),
            "\"conversationOptIn\""
        );
        assert_eq!(
            serde_json::to_string(&DataSourceAccessPolicy::AskWhenRelevant).unwrap(),
            "\"askWhenRelevant\""
        );
    }

    #[test]
    fn default_source_is_ready_but_conversation_can_disable_it() {
        let now = Utc::now();
        let ready = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(contract(now)),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert!(ready.enabled);
        assert!(ready.usable);
        assert_eq!(ready.reason, DataSourceResolutionReason::Ready);

        let disabled = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            conversation_override: Some(false),
            contract: Some(contract(now)),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert!(!disabled.enabled);
        assert_eq!(
            disabled.reason,
            DataSourceResolutionReason::DisabledForConversation
        );
    }

    #[test]
    fn opt_in_and_relevant_sources_do_not_leak_without_choice() {
        let opt_in = resolve_data_source(&DataSourceResolutionInput {
            policy: DataSourceAccessPolicy::ConversationOptIn,
            ..DataSourceResolutionInput::default()
        });
        assert!(!opt_in.enabled);
        assert!(!opt_in.should_request_consent);
        assert_eq!(
            opt_in.reason,
            DataSourceResolutionReason::ConversationOptInRequired
        );

        let relevant = resolve_data_source(&DataSourceResolutionInput {
            policy: DataSourceAccessPolicy::AskWhenRelevant,
            ..DataSourceResolutionInput::default()
        });
        assert!(!relevant.enabled);
        assert!(relevant.should_request_consent);
        assert_eq!(relevant.reason, DataSourceResolutionReason::ConsentRequired);
    }

    #[test]
    fn stale_and_failed_contracts_require_refresh() {
        let now = Utc::now();
        let mut stale = contract(now - Duration::minutes(61));
        stale.validated_at = now - Duration::minutes(61);
        let stale_result = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(stale),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert!(stale_result.enabled);
        assert!(!stale_result.usable);
        assert!(stale_result.should_refresh);
        assert_eq!(stale_result.reason, DataSourceResolutionReason::Stale);

        let mut failed = contract(now);
        failed.validation_state = DataSourceValidationState::Failed;
        let failed_result = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(failed),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert_eq!(
            failed_result.reason,
            DataSourceResolutionReason::ValidationFailed
        );
    }

    #[test]
    fn usage_provenance_round_trips_without_losing_cloud_disclosure() {
        let record = DataSourceUsageRecord {
            id: "usage-1".to_string(),
            source_id: "calendar".to_string(),
            source_title: "Calendar".to_string(),
            consumer: "chat".to_string(),
            purpose: "conversation_context".to_string(),
            conversation_id: Some("conversation-1".to_string()),
            used_at: Utc::now(),
            disclosed_to_cloud: Some(true),
            metadata: BTreeMap::new(),
        };
        let encoded = serde_json::to_vec(&record).unwrap();
        let decoded: DataSourceUsageRecord = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.disclosed_to_cloud, Some(true));
    }

    #[test]
    fn incomplete_or_tampered_reader_contract_requires_revalidation() {
        let now = Utc::now();
        let mut incomplete = contract(now);
        incomplete.read_recipe = DataSourceReadRecipe::default();
        incomplete.validation_digest.clear();
        let incomplete_result = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(incomplete),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert_eq!(
            incomplete_result.reason,
            DataSourceResolutionReason::ContractInvalid
        );
        assert!(incomplete_result.should_refresh);

        let mut tampered = contract(now);
        tampered.read_recipe.operation_id = "unvalidated.reader".to_string();
        let tampered_result = resolve_data_source(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(tampered),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        });
        assert_eq!(
            tampered_result.reason,
            DataSourceResolutionReason::ContractInvalid
        );
    }

    #[test]
    fn ffi_resolution_is_versioned() {
        let now = Utc::now();
        let bytes = serde_json::to_vec(&DataSourceResolutionInput {
            source_id: "calendar".to_string(),
            contract: Some(contract(now)),
            now: Some(now),
            ..DataSourceResolutionInput::default()
        })
        .unwrap();
        let pointer = nola_core_data_source_resolve(bytes.as_ptr(), bytes.len());
        assert!(!pointer.is_null());
        let json = unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() };
        crate::utils::nola_core_free_c_string(pointer);
        let output: DataSourceResolution = serde_json::from_str(&json).unwrap();
        assert_eq!(output.schema_version, DATA_SOURCE_SCHEMA_VERSION);
        assert!(output.usable);
    }
}
