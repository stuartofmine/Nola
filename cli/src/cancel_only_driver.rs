// SPDX-License-Identifier: Apache-2.0
// Cancellation control plane: deliberately has no model/tool/MCP driver.

use nola_core::agent::host::{
    AgentHostDriver, HostRequest, HostRequestEnvelope, HostResponse, HostResponseEnvelope,
};

/// Implemented by the cancellation delivery owner after the exact Run's
/// Interrupt is committed. Implementations must inspect durable target
/// ownership and either contact the live owner or prove its death. An empty
/// process-local registry is never sufficient evidence.
pub(super) trait CancellationAuthority {
    fn confirm_effect_quiescent(
        &mut self,
        request: &HostRequestEnvelope,
        target_effect_id: &str,
    ) -> Result<(), String>;

    /// Requests cancellation of this exact durable task; a successful request
    /// is not evidence that the task or parent Run is terminal.
    fn request_task_cancellation(
        &mut self,
        request: &HostRequestEnvelope,
        task_id: &str,
    ) -> Result<bool, String>;
}

pub(super) struct CancelOnlyDriver<A> {
    authority: A,
}

impl<A: CancellationAuthority> CancelOnlyDriver<A> {
    pub(super) fn new(authority: A) -> Self {
        Self { authority }
    }
}

impl<A: CancellationAuthority> AgentHostDriver for CancelOnlyDriver<A> {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        if !matches!(
            request.payload,
            HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }
        ) {
            return Err("cancel-only delivery refuses non-cancellation work".into());
        }
        let mut sequence = request
            .resume_after_sequence
            .map_or(0, |value| value.saturating_add(1));
        if request.delivery.is_some() && !request.attempt_started {
            emit_response(&request, sequence, HostResponse::ExecutionStarted, emit)?;
            sequence = sequence.saturating_add(1);
        }
        let payload = match &request.payload {
            HostRequest::Cancel { target_effect_id, .. } => {
                self.authority
                    .confirm_effect_quiescent(&request, &target_effect_id.0)?;
                HostResponse::CancellationConfirmed {
                    target_effect_id: target_effect_id.clone(),
                }
            }
            HostRequest::CancelTask { task_id } => {
                let requested = self
                    .authority
                    .request_task_cancellation(&request, task_id)?;
                HostResponse::Completed {
                    output: serde_json::json!({"task_id": task_id, "cancel_requested": requested}),
                    usage: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                }
            }
            _ => return Err("cancel-only delivery refuses non-cancellation work".into()),
        };
        emit_response(&request, sequence, payload, emit)
    }
}

fn emit_response(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
    emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
) -> Result<(), String> {
    emit(HostResponseEnvelope {
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
        delivery: request.delivery.clone(),
        sequence,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_core::agent::host::{HostEffectId, HostId};

    #[derive(Default)]
    struct Authority {
        confirmed: bool,
        effects: Vec<String>,
        tasks: Vec<String>,
    }

    impl CancellationAuthority for Authority {
        fn confirm_effect_quiescent(
            &mut self,
            _: &HostRequestEnvelope,
            target: &str,
        ) -> Result<(), String> {
            self.effects.push(target.into());
            if self.confirmed {
                Ok(())
            } else {
                Err("owner death is unproven".into())
            }
        }

        fn request_task_cancellation(
            &mut self,
            _: &HostRequestEnvelope,
            task: &str,
        ) -> Result<bool, String> {
            self.tasks.push(task.into());
            Ok(true)
        }
    }

    fn request(payload: HostRequest) -> HostRequestEnvelope {
        HostRequestEnvelope {
            attempt_started: false,
            attempt_id: Some("attempt".into()),
            delegated_invocation_id: None,
            request_id: "request".into(),
            effect_id: HostEffectId("cancel-effect".into()),
            host_id: HostId("host".into()),
            lease_id: Some("lease".into()),
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            catalog_revision: 7,
            availability_epoch: 8,
            created_at_ms: 9,
            deadline_ms: None,
            batch: None,
            resume_after_sequence: Some(11),
            delivery: None,
            payload,
        }
    }

    #[test]
    fn unknown_or_live_owner_never_acknowledges_cancellation() {
        let mut driver = CancelOnlyDriver::new(Authority::default());
        let mut responses = Vec::new();
        let result = driver.dispatch(
            request(HostRequest::Cancel {
                target_effect_id: HostEffectId("original".into()),
                target_not_started: None,
            }),
            &mut |response| {
                responses.push(response);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(responses.is_empty());
        assert_eq!(driver.authority.effects, ["original"]);
    }

    #[test]
    fn proven_target_preserves_exact_response_identity_and_sequence() {
        let mut driver = CancelOnlyDriver::new(Authority {
            confirmed: true,
            ..Default::default()
        });
        let original = request(HostRequest::Cancel {
            target_effect_id: HostEffectId("original".into()),
            target_not_started: None,
        });
        let mut responses = Vec::new();
        driver
            .dispatch(original.clone(), &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        let response = &responses[0];
        assert_eq!(response.request_id, original.request_id);
        assert_eq!(response.attempt_id, original.attempt_id);
        assert_eq!(response.effect_id, original.effect_id);
        assert_eq!(response.session_id, original.session_id);
        assert_eq!(response.turn_id, original.turn_id);
        assert_eq!(response.lease_id, original.lease_id);
        assert_eq!(response.sequence, 12);
        assert!(
            matches!(&response.payload, HostResponse::CancellationConfirmed { target_effect_id } if target_effect_id.0 == "original")
        );
    }

    #[test]
    fn task_cancel_is_only_a_request_not_parent_cancellation_confirmation() {
        let mut driver = CancelOnlyDriver::new(Authority::default());
        let mut responses = Vec::new();
        driver
            .dispatch(
                request(HostRequest::CancelTask {
                    task_id: "task".into(),
                }),
                &mut |response| {
                    responses.push(response);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(driver.authority.tasks, ["task"]);
        assert!(driver.authority.effects.is_empty());
        assert!(matches!(
            responses[0].payload,
            HostResponse::Completed { .. }
        ));
    }

    #[test]
    fn non_control_work_never_reaches_authority_or_emits_response() {
        let mut driver = CancelOnlyDriver::new(Authority::default());
        let mut emitted = 0;
        let result = driver.dispatch(
            request(HostRequest::RefreshCapabilityCatalog {
                run_id: "run".into(),
                catalog_revision: 1,
                errors: Vec::new(),
            }),
            &mut |_| {
                emitted += 1;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(emitted, 0);
        assert!(driver.authority.effects.is_empty() && driver.authority.tasks.is_empty());
    }
}
