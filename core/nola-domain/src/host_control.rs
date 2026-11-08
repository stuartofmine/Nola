// SPDX-License-Identifier: Apache-2.0
//! Bounded reducer authority, separate from immutable execution payloads.
use crate::host_effect::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn permission_request() -> HostRequestEnvelope<HostRequest> {
        serde_json::from_value(json!({
            "request_id":"permission-request","effect_id":"permission-effect","host_id":"host","lease_id":null,
            "session_id":"session","turn_id":"turn","catalog_revision":1,"availability_epoch":1,
            "created_at_ms":1,"deadline_ms":null,
            "payload":{"type":"resolve_delegated_permission",
                "occurrence":{"occurrence_id":"occurrence","worker_generation":"worker"},
                "target_effect_id":"delegated-effect","backend_id":"backend","backend_session_id":"backend-session",
                "permission_request_id":"rpc-request","resolution":{"type":"authorization","decision":"allow_once","grant":null}}
        })).unwrap()
    }

    #[test]
    fn permission_delivery_is_the_only_mutable_payload_binding_and_rebind_is_atomic() {
        let mut request = permission_request();
        let mut control = HostEffectControl::from_request(&request).unwrap();
        let identity = control.payload.payload_identity.clone();
        let target = DelegatedPermissionDeliveryTarget {
            delivery_id: "delivery".into(),
            invocation_id: "invocation".into(),
            worker_generation: "worker".into(),
            occurrence_id: "occurrence".into(),
            call_id: "call".into(),
            request_digest: "digest".into(),
        };
        let mut binding = control.binding();
        binding.payload_binding = CoreHostPayloadBinding::DelegatedPermissionDelivery {
            delivery: Some(target.clone()),
        };
        control.apply_binding(&binding).unwrap();
        binding.apply_to(&mut request).unwrap();
        assert_eq!(control, HostEffectControl::from_request(&request).unwrap());
        assert_eq!(control.payload.payload_identity, identity);
        let before = control.clone();
        binding.request_id = "different-request".into();
        let mut foreign = target;
        foreign.occurrence_id = "another-occurrence".into();
        binding.payload_binding = CoreHostPayloadBinding::DelegatedPermissionDelivery {
            delivery: Some(foreign),
        };
        assert!(control.apply_binding(&binding).is_err());
        assert_eq!(control, before);
        binding = control.binding();
        binding.turn_id = Some("another-turn".into());
        assert!(control.apply_binding(&binding).is_err());
        assert_eq!(control, before);
        binding = control.binding();
        binding.request_id = "x".repeat(HOST_EFFECT_CONTROL_MAX_BYTES + 1);
        assert!(control.apply_binding(&binding).is_err());
        assert_eq!(control, before);
        let mut changed = permission_request();
        if let HostRequest::ResolveDelegatedPermission { resolution, .. } = &mut changed.payload {
            *resolution = HostPermissionResolution::deny();
        }
        assert_ne!(
            HostEffectControl::from_request(&changed)
                .unwrap()
                .payload
                .payload_identity,
            identity
        );
    }

    #[test]
    fn control_construction_rejects_oversized_identity_without_truncation() {
        let mut request = permission_request();
        request.request_id = "x".repeat(HOST_EFFECT_CONTROL_MAX_BYTES + 1);
        assert!(HostEffectControl::from_request(&request).is_err());
        assert_eq!(request.request_id.len(), HOST_EFFECT_CONTROL_MAX_BYTES + 1);
    }
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreHostPayloadBinding {
    Unchanged,
    CancellationNotStarted {
        #[serde(deserialize_with = "required_option")]
        proof: Option<crate::cancellation_not_started::CancellationTargetNotStarted>,
    },
    DelegatedPermissionDelivery {
        #[serde(deserialize_with = "required_option")]
        delivery: Option<DelegatedPermissionDeliveryTarget>,
    },
}

/// All envelope metadata consumed by an actual Core bind. Optional fields are
/// required even when null: missing historical evidence is not a null value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreHostBindingOverlay {
    pub attempt_started: bool,
    #[serde(deserialize_with = "required_option")]
    pub attempt_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub delegated_invocation_id: Option<String>,
    pub request_id: String,
    pub effect_id: HostEffectId,
    pub host_id: HostId,
    #[serde(deserialize_with = "required_option")]
    pub lease_id: Option<String>,
    pub session_id: String,
    #[serde(deserialize_with = "required_option")]
    pub turn_id: Option<String>,
    pub catalog_revision: u64,
    pub availability_epoch: u64,
    pub created_at_ms: u64,
    #[serde(deserialize_with = "required_option")]
    pub deadline_ms: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    pub batch: Option<HostRequestBatchIdentity>,
    #[serde(deserialize_with = "required_option")]
    pub resume_after_sequence: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    pub delivery: Option<EffectDeliveryLease>,
    pub payload_binding: CoreHostPayloadBinding,
}

impl CoreHostBindingOverlay {
    pub fn from_request<P, E, L>(
        request: &crate::host_effect::HostRequestEnvelope<crate::host_effect::HostRequest<P, E, L>>,
    ) -> Self {
        Self {
            attempt_started: request.attempt_started,
            attempt_id: request.attempt_id.clone(),
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            created_at_ms: request.created_at_ms,
            deadline_ms: request.deadline_ms,
            batch: request.batch.clone(),
            resume_after_sequence: request.resume_after_sequence,
            delivery: request.delivery.clone(),
            payload_binding: match &request.payload {
                crate::host_effect::HostRequest::Cancel { target_not_started, .. } =>
                    CoreHostPayloadBinding::CancellationNotStarted { proof: target_not_started.clone() },
                crate::host_effect::HostRequest::ResolveDelegatedPermission {
                    delivery, ..
                } => CoreHostPayloadBinding::DelegatedPermissionDelivery {
                    delivery: delivery.clone(),
                },
                _ => CoreHostPayloadBinding::Unchanged,
            },
        }
    }

    pub fn apply_to<P, E, L>(
        &self,
        request: &mut crate::host_effect::HostRequestEnvelope<
            crate::host_effect::HostRequest<P, E, L>,
        >,
    ) -> Result<(), String> {
        if request.effect_id != self.effect_id
            || request.session_id != self.session_id
            || request.turn_id != self.turn_id
        {
            return Err("Host binding changed immutable effect/session/Turn ownership".into());
        }
        if request.delegated_invocation_id.is_some()
            && request.delegated_invocation_id != self.delegated_invocation_id
        {
            return Err("Host binding changed immutable delegated invocation".into());
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len()
            > HOST_EFFECT_CONTROL_MAX_BYTES
        {
            return Err("Host binding exceeds bounded metadata limit".into());
        }
        match (&mut request.payload, &self.payload_binding) {
            (crate::host_effect::HostRequest::Cancel { target_effect_id, target_not_started },
                CoreHostPayloadBinding::CancellationNotStarted { proof }) => {
                if target_not_started.is_some() && target_not_started != proof {
                    return Err("Host binding replaced a cancellation NotStarted proof".into());
                }
                if proof.as_ref().is_some_and(|proof| !proof.matches_control(&self.host_id.0,
                    &self.session_id, self.turn_id.as_deref(), &self.effect_id.0,
                    &self.request_id, &target_effect_id.0)) {
                    return Err("Host binding has foreign cancellation NotStarted proof".into());
                }
                *target_not_started = proof.clone();
            }
            (crate::host_effect::HostRequest::Cancel { .. }, _)
            | (_, CoreHostPayloadBinding::CancellationNotStarted { .. }) =>
                return Err("Host binding cancellation payload kind mismatch".into()),
            (
                crate::host_effect::HostRequest::ResolveDelegatedPermission { delivery, .. },
                CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery: admitted },
            ) => {
                if delivery.is_some() && delivery != admitted {
                    return Err(
                        "Host binding changed immutable delegated permission delivery".into(),
                    );
                }
                *delivery = admitted.clone();
            }
            (
                crate::host_effect::HostRequest::ResolveDelegatedPermission { .. },
                CoreHostPayloadBinding::Unchanged,
            )
            | (_, CoreHostPayloadBinding::DelegatedPermissionDelivery { .. }) => {
                return Err("Host binding payload kind mismatch".into())
            }
            _ => {}
        }
        request.attempt_started = self.attempt_started;
        request.attempt_id = self.attempt_id.clone();
        request.delegated_invocation_id = self.delegated_invocation_id.clone();
        request.request_id = self.request_id.clone();
        request.host_id = self.host_id.clone();
        request.lease_id = self.lease_id.clone();
        request.catalog_revision = self.catalog_revision;
        request.availability_epoch = self.availability_epoch;
        request.created_at_ms = self.created_at_ms;
        request.deadline_ms = self.deadline_ms;
        request.batch = self.batch.clone();
        request.resume_after_sequence = self.resume_after_sequence;
        request.delivery = self.delivery.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostResponsePolicy {
    PollCodeCell { cell: crate::code_mode::CodeCellIdentity },
    ResolveCodeTool {
        cell: crate::code_mode::CodeCellIdentity,
        sequence: u32,
    },
    ResolvePermissionResources { call_id: String },
    ModelStream {
        response_mode: ModelResponseMode,
    },
    DelegatedAgent {},
    ExecuteTool {
        call_id: String,
    },
    RequestPermission {
        permission_request_id: String,
    },
    ResolveDelegatedPermission {
        occurrence: DelegatedPermissionOccurrence,
        delivery: Option<DelegatedPermissionDeliveryTarget>,
        target_effect_id: HostEffectId,
        backend_id: String,
        backend_session_id: Option<String>,
        permission_request_id: String,
        resolution: HostPermissionResolution,
    },
    FetchContext {},
    Compact {},
    Cancel {
        target_effect_id: HostEffectId,
        #[serde(deserialize_with = "required_option")]
        target_not_started: Option<crate::cancellation_not_started::CancellationTargetNotStarted>,
    },
    CancelTask {},
    RefreshCapabilityCatalog {},
    RunLifecycleHook {},
}
impl HostResponsePolicy {
    pub fn from_request<P, E, L>(request: &HostRequest<P, E, L>) -> Self {
        match request {
            HostRequest::PollCodeCell { cell } => Self::PollCodeCell { cell:cell.clone() },
            HostRequest::ResolveCodeTool { result } => Self::ResolveCodeTool {
                cell: result.cell.clone(), sequence: result.sequence,
            },
            HostRequest::ResolvePermissionResources { call_id, .. } => Self::ResolvePermissionResources { call_id: call_id.clone() },
            HostRequest::ModelStream { response_mode, .. } => Self::ModelStream {
                response_mode: *response_mode,
            },
            HostRequest::DelegatedAgent { .. } => Self::DelegatedAgent {},
            HostRequest::ExecuteTool { call_id, .. } => Self::ExecuteTool {
                call_id: call_id.clone(),
            },
            HostRequest::StartCodeCell { identity, .. } => Self::ExecuteTool {
                call_id: identity.parent_call_id.clone(),
            },
            HostRequest::ObserveCodeCell { call_id, .. }
                | HostRequest::ReadCodeCellHistory { call_id, .. } => Self::ExecuteTool {
                call_id: call_id.clone(),
            },
            HostRequest::RequestPermission {
                permission_request_id,
                ..
            } => Self::RequestPermission {
                permission_request_id: permission_request_id.clone(),
            },
            HostRequest::ResolveDelegatedPermission {
                occurrence,
                delivery,
                target_effect_id,
                backend_id,
                backend_session_id,
                permission_request_id,
                resolution,
                ..
            } => Self::ResolveDelegatedPermission {
                occurrence: occurrence.clone(),
                delivery: delivery.clone(),
                target_effect_id: target_effect_id.clone(),
                backend_id: backend_id.clone(),
                backend_session_id: backend_session_id.clone(),
                permission_request_id: permission_request_id.clone(),
                resolution: resolution.clone(),
            },
            HostRequest::FetchContext { .. } => Self::FetchContext {},
            HostRequest::Compact { .. } => Self::Compact {},
            HostRequest::Cancel { target_effect_id, target_not_started } => Self::Cancel {
                target_effect_id: target_effect_id.clone(),
                target_not_started: target_not_started.clone(),
            },
            HostRequest::CancelTask { .. } => Self::CancelTask {},
            HostRequest::RefreshCapabilityCatalog { .. } => Self::RefreshCapabilityCatalog {},
            HostRequest::RunLifecycleHook { .. } => Self::RunLifecycleHook {},
        }
    }
    /// Validates the response family before a reducer applies it.
    pub fn accepts_response<
        TContinuation,
        TTaskKind,
        TTaskState,
        TTaskLineage,
        TCapabilityCatalog,
        TLifecycleProjection,
    >(
        &self,
        response: &HostResponse<
            TContinuation,
            TTaskKind,
            TTaskState,
            TTaskLineage,
            TCapabilityCatalog,
            TLifecycleProjection,
        >,
    ) -> bool {
        if matches!(
            response,
            HostResponse::Accepted
                | HostResponse::ExecutionStarted
                | HostResponse::ExecutionRejected { .. }
                | HostResponse::Failed { .. }
        ) {
            return true;
        }
        match self {
            Self::ResolvePermissionResources { call_id } => matches!(response,
                HostResponse::PermissionResourcesResolved { call_id: received, .. } if received == call_id),
            Self::ModelStream { .. } => matches!(
                response,
                HostResponse::ModelAttemptStarted { .. }
                    | HostResponse::ModelUsageReported { .. }
                    | HostResponse::ModelDelta { .. }
                    | HostResponse::ModelCommittedDelta { .. }
                    | HostResponse::ModelToolCallDelta { .. }
                    | HostResponse::ModelContinuation { .. }
                    | HostResponse::ToolProgress { .. }
                    | HostResponse::ModelCompleted { .. }
                    | HostResponse::ModelOutputLimited { .. }
            ),
            Self::DelegatedAgent { .. } => {
                matches!(
                    response,
                    HostResponse::DelegatedAgentEvent { .. }
                        | HostResponse::DelegatedExecutionInterrupted { .. }
                )
            }
            Self::ExecuteTool { call_id, .. } => match response {
                HostResponse::CodeModeToolRequested { request } =>
                    !call_id.is_empty() && request.cell.parent_call_id == *call_id,
                HostResponse::CodeModeOutput { frame } =>
                    !call_id.is_empty() && frame.cell.parent_call_id == *call_id,
                HostResponse::CodeModeYielded { cell, .. } =>
                    !call_id.is_empty() && cell.parent_call_id == *call_id,
                HostResponse::CodeModeObservationSuspended { call_id: received, .. } =>
                    !call_id.is_empty() && received == call_id,
                HostResponse::ToolCallProgress {
                    call_id: received, ..
                }
                | HostResponse::ToolCompleted {
                    call_id: received, ..
                } => !call_id.is_empty() && received == call_id,
                HostResponse::ToolProgress { .. }
                | HostResponse::TaskPromoted { .. }
                | HostResponse::TaskUpdated { .. } => true,
                _ => false,
            },
            Self::PollCodeCell { cell } => match response {
                HostResponse::Completed { output, usage, full_output_ref, artifacts } =>
                    output == &serde_json::json!({"cell":cell})
                        && usage.is_none() && full_output_ref.is_none() && artifacts.is_empty(),
                _ => false,
            },
            Self::ResolveCodeTool { cell, sequence } => match response {
                HostResponse::Completed { output, usage, full_output_ref, artifacts } =>
                    output == &serde_json::json!({"cell":cell,"sequence":sequence})
                        && usage.is_none() && full_output_ref.is_none() && artifacts.is_empty(),
                _ => false,
            },
            Self::RequestPermission {
                permission_request_id,
                ..
            } => match response {
                HostResponse::PermissionResolved { request_id, .. } => {
                    !permission_request_id.is_empty() && request_id == permission_request_id
                }
                HostResponse::PermissionTransportInterrupted { .. } => true,
                _ => false,
            },
            Self::ResolveDelegatedPermission { .. } => {
                matches!(
                    response,
                    HostResponse::DelegatedPermissionClaimed { .. }
                        | HostResponse::DelegatedPermissionTransportWritten { .. }
                        | HostResponse::DelegatedPermissionInterrupted { .. }
                )
            }
            Self::FetchContext { .. } => matches!(response, HostResponse::ContextFetched { .. }),
            Self::Compact { .. } => {
                matches!(
                    response,
                    HostResponse::CompactionCompleted { .. }
                        | HostResponse::ModelAttemptStarted { .. }
                        | HostResponse::ModelUsageReported { .. }
                        | HostResponse::ToolProgress { .. }
                )
            }
            Self::CancelTask { .. } => matches!(
                response,
                HostResponse::Completed { .. } | HostResponse::TaskUpdated { .. }
            ),
            Self::Cancel { .. } => {
                matches!(response, HostResponse::CancellationConfirmed { .. })
            }
            Self::RefreshCapabilityCatalog { .. } => {
                matches!(response, HostResponse::CapabilityCatalogRefreshed { .. })
            }
            Self::RunLifecycleHook { .. } => matches!(
                response,
                HostResponse::LifecycleHookUpdated { .. } | HostResponse::Completed { .. }
            ),
        }
    }
}

/// An unavailable permission transport is not a user authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionTransportInterruptionReason {
    Timeout,
    Disconnected,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRequestControl {
    pub policy: HostResponsePolicy,
    /// Digest of the immutable request body, calculated only at construction/restore.
    pub payload_identity: String,
}
pub type HostEffectControl = HostRequestEnvelope<HostRequestControl>;
pub const HOST_EFFECT_CONTROL_MAX_BYTES: usize = 32 * 1024;
impl HostRequestControl {
    pub fn accepts_response<A, B, C, D, E, F>(
        &self,
        response: &HostResponse<A, B, C, D, E, F>,
    ) -> bool {
        self.policy.accepts_response(response)
    }
}
impl HostEffectControl {
    pub fn from_request<P: Serialize, E: Serialize, L: Serialize>(
        request: &HostRequestEnvelope<HostRequest<P, E, L>>,
    ) -> Result<Self, String> {
        use sha2::{Digest, Sha256};
        let mut payload = serde_json::to_value(&request.payload).map_err(|e| e.to_string())?;
        if matches!(request.payload, HostRequest::Cancel { .. }) {
            payload.as_object_mut().ok_or("cancel payload is not an object")?.remove("target_not_started");
        }
        if matches!(
            request.payload,
            HostRequest::ResolveDelegatedPermission { .. }
        ) {
            payload
                .as_object_mut()
                .ok_or("permission payload is not an object")?
                .remove("delivery");
        }
        let payload_identity = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
        );
        let control = Self {
            attempt_started: request.attempt_started,
            attempt_id: request.attempt_id.clone(),
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            created_at_ms: request.created_at_ms,
            deadline_ms: request.deadline_ms,
            batch: request.batch.clone(),
            resume_after_sequence: request.resume_after_sequence,
            delivery: request.delivery.clone(),
            payload: HostRequestControl {
                policy: HostResponsePolicy::from_request(&request.payload),
                payload_identity,
            },
        };
        control.validate_bound()?;
        Ok(control)
    }
    pub fn validate_bound(&self) -> Result<(), String> {
        if let HostResponsePolicy::Cancel { target_effect_id, target_not_started: Some(proof) } = &self.payload.policy {
            if !proof.matches_control(&self.host_id.0, &self.session_id, self.turn_id.as_deref(),
                &self.effect_id.0, &self.request_id, &target_effect_id.0) {
                return Err("Host effect control has an invalid cancellation NotStarted proof".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len()
            > HOST_EFFECT_CONTROL_MAX_BYTES
        {
            return Err("Host effect control exceeds bounded metadata limit".into());
        }
        Ok(())
    }
    pub fn binding(&self) -> CoreHostBindingOverlay {
        CoreHostBindingOverlay {
            attempt_started: self.attempt_started,
            attempt_id: self.attempt_id.clone(),
            delegated_invocation_id: self.delegated_invocation_id.clone(),
            request_id: self.request_id.clone(),
            effect_id: self.effect_id.clone(),
            host_id: self.host_id.clone(),
            lease_id: self.lease_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            catalog_revision: self.catalog_revision,
            availability_epoch: self.availability_epoch,
            created_at_ms: self.created_at_ms,
            deadline_ms: self.deadline_ms,
            batch: self.batch.clone(),
            resume_after_sequence: self.resume_after_sequence,
            delivery: self.delivery.clone(),
            payload_binding: match &self.payload.policy {
                HostResponsePolicy::Cancel { target_not_started, .. } =>
                    CoreHostPayloadBinding::CancellationNotStarted { proof: target_not_started.clone() },
                HostResponsePolicy::ResolveDelegatedPermission { delivery, .. } => {
                    CoreHostPayloadBinding::DelegatedPermissionDelivery {
                        delivery: delivery.clone(),
                    }
                }
                _ => CoreHostPayloadBinding::Unchanged,
            },
        }
    }
    pub fn apply_binding(&mut self, binding: &CoreHostBindingOverlay) -> Result<(), String> {
        if self.effect_id != binding.effect_id
            || self.session_id != binding.session_id
            || self.turn_id != binding.turn_id
        {
            return Err("Host binding changed immutable effect/session/Turn ownership".into());
        }
        if self.delegated_invocation_id.is_some()
            && self.delegated_invocation_id != binding.delegated_invocation_id
        {
            return Err("Host binding changed immutable delegated invocation".into());
        }
        let mut next = self.clone();
        match (&mut next.payload.policy, &binding.payload_binding) {
            (HostResponsePolicy::Cancel { target_effect_id, target_not_started },
                CoreHostPayloadBinding::CancellationNotStarted { proof }) => {
                if target_not_started.is_some() && target_not_started != proof {
                    return Err("Host binding replaced a cancellation NotStarted proof".into());
                }
                if proof.as_ref().is_some_and(|proof| !proof.matches_control(&binding.host_id.0,
                    &binding.session_id, binding.turn_id.as_deref(), &binding.effect_id.0,
                    &binding.request_id, &target_effect_id.0)) {
                    return Err("Host binding has foreign cancellation NotStarted proof".into());
                }
                *target_not_started = proof.clone();
            }
            (HostResponsePolicy::Cancel { .. }, _)
            | (_, CoreHostPayloadBinding::CancellationNotStarted { .. }) =>
                return Err("Host binding cancellation policy kind mismatch".into()),
            (
                HostResponsePolicy::ResolveDelegatedPermission { delivery, .. },
                CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery: admitted },
            ) => {
                if delivery.is_some() && delivery != admitted {
                    return Err(
                        "Host binding changed immutable delegated permission delivery".into(),
                    );
                }
                *delivery = admitted.clone();
            }
            (
                HostResponsePolicy::ResolveDelegatedPermission { .. },
                CoreHostPayloadBinding::Unchanged,
            )
            | (_, CoreHostPayloadBinding::DelegatedPermissionDelivery { .. }) => {
                return Err("Host binding payload kind mismatch".into())
            }
            _ => {}
        }
        next.attempt_started = binding.attempt_started;
        next.attempt_id = binding.attempt_id.clone();
        next.delegated_invocation_id = binding.delegated_invocation_id.clone();
        next.request_id = binding.request_id.clone();
        next.host_id = binding.host_id.clone();
        next.lease_id = binding.lease_id.clone();
        next.catalog_revision = binding.catalog_revision;
        next.availability_epoch = binding.availability_epoch;
        next.created_at_ms = binding.created_at_ms;
        next.deadline_ms = binding.deadline_ms;
        next.batch = binding.batch.clone();
        next.resume_after_sequence = binding.resume_after_sequence;
        next.delivery = binding.delivery.clone();
        next.validate_bound()?;
        *self = next;
        Ok(())
    }
}
