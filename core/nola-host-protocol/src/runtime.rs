// SPDX-License-Identifier: Apache-2.0
//! The execution pump's complete view of a Core implementation. Payloads are
//! opaque except for transport lifecycle facts; no reducer or policy is exposed.

use crate::{HostRequestEnvelope, HostResponseEnvelope};

pub trait HostExecutionProtocol: Clone + std::fmt::Debug {
    type Request: RuntimeRequestPayload + Send;
    type Response: RuntimeResponsePayload + Send;
    type BoundaryInput: Clone;
    type Event: Clone + std::fmt::Debug;
    type Trace: Clone + std::fmt::Debug;
    type Control: Clone + std::fmt::Debug;
}

/// Cell receipts must drain before a synchronous Host waits for a person.
/// This orders delivery only; it does not authorize or replay nested work.
pub fn prioritize_code_cell_controls<P, E, L>(
    requests: &mut [HostRequestEnvelope<crate::HostRequest<P, E, L>>],
) {
    if requests.iter().any(|request| matches!(&request.payload, crate::HostRequest::RequestPermission { .. })) {
        requests.sort_by_key(|request| !matches!(&request.payload,
            crate::HostRequest::ResolveCodeTool { .. } | crate::HostRequest::PollCodeCell { .. }));
    }
}

pub trait RuntimeRequestPayload {
    fn permission_wait_id(&self) -> Option<&str>;
    fn cancellation_target_effect_id(&self) -> Option<&str>;
}

pub trait RuntimeResponsePayload {
    fn is_model_delta(&self) -> bool;
    fn execution_started(&self) -> bool;
    fn is_terminal(&self) -> bool;
    fn permission_suspension_id(&self) -> Option<&str>;
    fn code_cell_suspension_id(&self) -> Option<&str>;
}

impl<P, E, L> RuntimeRequestPayload for crate::HostRequest<P, E, L> {
    fn cancellation_target_effect_id(&self) -> Option<&str> {
        match self { Self::Cancel { target_effect_id, .. } => Some(&target_effect_id.0), _ => None }
    }
    fn permission_wait_id(&self) -> Option<&str> {
        match self {
            Self::RequestPermission { permission_request_id, .. } => Some(permission_request_id),
            _ => None,
        }
    }
}

impl<C, K, S: crate::HostTaskState, L, A, H> RuntimeResponsePayload
    for crate::HostResponse<C, K, S, L, A, H>
{
    fn is_model_delta(&self) -> bool { matches!(self, Self::ModelDelta { .. }) }
    fn execution_started(&self) -> bool { matches!(self, Self::ExecutionStarted) }
    fn is_terminal(&self) -> bool { crate::HostResponse::is_terminal(self) }
    fn permission_suspension_id(&self) -> Option<&str> {
        match self {
            Self::DelegatedAgentEvent {
                event: crate::DelegatedAgentEventV1::PermissionRequested { occurrence, .. },
            } => Some(&occurrence.occurrence_id),
            _ => None,
        }
    }
    fn code_cell_suspension_id(&self) -> Option<&str> {
        match self {
            Self::CodeModeToolRequested { request } => Some(&request.cell.cell_id),
            Self::CodeModeYielded { cell, .. } => Some(&cell.cell_id),
            Self::CodeModeObservationSuspended { cell, .. } => Some(&cell.cell_id),
            _ => None,
        }
    }
}

/// Application accepts each item durably before acknowledging its source.
/// This interface does not authorize execution or mutate Core state itself.
pub trait BoundaryInputSource<I>: Send + Sync {
    fn take_pending(&self) -> Result<Vec<I>, String>;
    fn did_apply(&self, input: &I) -> Result<(), String>;
    fn requeue(&self, input: &I) -> Result<(), String>;
}

/// Physical executor. Every response, including execution-start, must receive
/// Application's commit ACK before execution or its delivery sequence advances.
pub trait HostDriver {
    type Protocol: HostExecutionProtocol;
    type Error;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope<<Self::Protocol as HostExecutionProtocol>::Request>,
        emit: &mut dyn FnMut(HostResponseEnvelope<<Self::Protocol as HostExecutionProtocol>::Response>)
            -> Result<(), String>,
    ) -> Result<(), Self::Error>;

    fn dispatch_batch(
        &mut self,
        requests: Vec<HostRequestEnvelope<<Self::Protocol as HostExecutionProtocol>::Request>>,
        emit: &mut dyn FnMut(HostResponseEnvelope<<Self::Protocol as HostExecutionProtocol>::Response>)
            -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        for request in requests { self.dispatch(request, emit)?; }
        Ok(())
    }

    fn boundary_submission_source(
        &self,
    ) -> Option<std::sync::Arc<dyn BoundaryInputSource<<Self::Protocol as HostExecutionProtocol>::BoundaryInput>>> {
        None
    }
}
