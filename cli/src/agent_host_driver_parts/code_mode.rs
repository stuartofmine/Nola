// SPDX-License-Identifier: Apache-2.0
// Core owns tool admission; this adapter only hosts computation and receipts.

struct HostCodeCell {
    request: HostRequestEnvelope,
    worker: nola_code_mode_host::worker::CellWorker,
    sequence: u64,
    output: Vec<String>,
    completed: bool,
    awaiting: std::collections::BTreeSet<u32>,
    observations: Vec<HostCodeObservation>,
}

struct HostCodeObservation {
    request: HostRequestEnvelope,
    call_id: String,
    cursor: u64,
    deadline: std::time::Instant,
    sequence: u64,
}

#[cfg(test)]
mod code_observation_tests {
    use super::*;
    use super::effect_delivery_start_barrier_tests::{fenced_request, test_context, test_llm};

    #[test]
    fn suspended_observation_survives_child_dispatch_and_completes_once() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()));
        let owner = nola_domain::code_mode::CodeCellIdentity {
            host_id: "delivery-host".into(), conversation_id: "session-fenced".into(),
            turn_id: "turn-fenced".into(), parent_call_id: "parent".into(),
            cell_id: "cell".into(), generation: "generation".into(),
        };
        let request = fenced_request(&driver, HostRequest::PollCodeCell { cell: owner.clone() });
        let mut responses = Vec::new();
        driver.start_code_cell(&request, owner.clone(), Default::default(), 0,
            "const r = await tools.read({}); text(r);".into(),
            vec![nola_domain::code_mode::CodeToolDescription {
                name: "read".into(), description: "Read fixture".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }], &mut |response| { responses.push(response); Ok(()) }).unwrap();
        let mut observer_request = request.clone();
        observer_request.effect_id.0 = "observer-effect".into();
        observer_request.request_id = "observer-request".into();
        driver.observe_code_cell(&observer_request, "observer".into(), nola_domain::code_mode::CodeCellObservation {
            cell: owner.clone(), after_output_sequence: 0, wait_ms: 1000, terminate: false,
        }, &mut |response| { responses.push(response); Ok(()) }).unwrap();
        assert!(responses.iter().any(|response| matches!(response.payload, HostResponse::CodeModeObservationSuspended { .. })));
        assert!(!responses.iter().any(|response| matches!(&response.payload, HostResponse::ToolCompleted { call_id, .. } if call_id == "observer")));
        let original_deadline = driver.code_cells["cell"].observations[0].deadline;
        driver.observe_code_cell(&observer_request, "observer".into(), nola_domain::code_mode::CodeCellObservation {
            cell: owner.clone(), after_output_sequence: 0, wait_ms: 60000, terminate: false,
        }, &mut |response| { responses.push(response); Ok(()) }).unwrap();
        assert_eq!(driver.code_cells["cell"].observations.len(), 1, "redelivery must not duplicate the observer");
        assert_eq!(driver.code_cells["cell"].observations[0].deadline, original_deadline,
            "redelivery must not extend the original observation deadline");
        let mut timeout_request = observer_request.clone();
        timeout_request.effect_id.0 = "timeout-effect".into();
        timeout_request.request_id = "timeout-request".into();
        driver.observe_code_cell(&timeout_request, "timeout-observer".into(), nola_domain::code_mode::CodeCellObservation {
            cell: owner.clone(), after_output_sequence: 0, wait_ms: 1000, terminate: false,
        }, &mut |response| { responses.push(response); Ok(()) }).unwrap();
        driver.code_cells.get_mut("cell").unwrap().observations[1].deadline = std::time::Instant::now();
        driver.settle_code_observations("cell", &mut |response| { responses.push(response); Ok(()) }).unwrap();
        assert_eq!(responses.iter().filter(|response| matches!(&response.payload,
            HostResponse::ToolCompleted { call_id, .. } if call_id == "timeout-observer")).count(), 1);
        assert!(!driver.code_cells["cell"].completed, "observation timeout must not terminate computation");
        assert_eq!(driver.code_cells["cell"].observations.len(), 1, "timeout must not complete sibling observer");
        let mut cancelled_request = observer_request.clone();
        cancelled_request.effect_id.0 = "cancelled-observer-effect".into();
        cancelled_request.request_id = "cancelled-observer-request".into();
        driver.observe_code_cell(&cancelled_request, "cancelled-observer".into(), nola_domain::code_mode::CodeCellObservation {
            cell: owner.clone(), after_output_sequence: 0, wait_ms: 1000, terminate: false,
        }, &mut |response| { responses.push(response); Ok(()) }).unwrap();
        let mut foreign_control = request.clone();
        foreign_control.session_id = "foreign".into();
        assert!(driver.stop_code_cell(&foreign_control, "cancelled-observer-effect").is_err());
        assert_eq!(driver.code_cells["cell"].observations.len(), 2);
        assert!(driver.stop_code_cell(&request, "cancelled-observer-effect").unwrap().is_some());
        assert_eq!(driver.code_cells["cell"].observations.len(), 1);
        assert!(!driver.code_cells["cell"].completed, "withdrawing an observer must not stop its cell");
        let cell = driver.code_cells.get_mut("cell").unwrap();
        let deadline = cell.observations[0].deadline;
        cell.worker.resolve(nola_domain::code_mode::CodeToolResult {
            cell: owner, sequence: 1, output: serde_json::json!({"total":42}), is_error: false,
        }).unwrap();
        cell.awaiting.remove(&1);
        driver.drain_code_cell("cell", &mut |response| { responses.push(response); Ok(()) }).unwrap();
        assert_eq!(driver.code_cells["cell"].observations[0].deadline, deadline);
        let pending_sequence = driver.code_cells["cell"].observations[0].sequence;
        assert!(driver.settle_code_observations("cell", &mut |_| Err("receipt transport failed".into())).is_err());
        assert_eq!(driver.code_cells["cell"].observations.len(), 1,
            "an unacknowledged completion must remain available for retry");
        assert_eq!(driver.code_cells["cell"].observations[0].sequence, pending_sequence,
            "retry must use the same response sequence, not skip an uncommitted receipt");
        for _ in 0..2 {
            driver.settle_code_observations("cell", &mut |response| { responses.push(response); Ok(()) }).unwrap();
        }
        assert_eq!(responses.iter().filter(|response| matches!(&response.payload,
            HostResponse::ToolCompleted { call_id, .. } if call_id == "observer")).count(), 1);
        assert!(driver.code_cells["cell"].observations.is_empty());
    }

    #[test]
    fn missing_worker_rejects_observation_and_source_redelivery() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()));
        let owner = nola_domain::code_mode::CodeCellIdentity {
            host_id:"delivery-host".into(),conversation_id:"session-fenced".into(),
            turn_id:"turn-fenced".into(),parent_call_id:"parent".into(),
            cell_id:"lost-cell".into(),generation:"original-generation".into(),
        };
        let mut request = fenced_request(&driver, HostRequest::PollCodeCell {cell:owner.clone()});
        let mut responses = Vec::new();
        let error = driver.observe_code_cell(&request,"observe".into(),
            nola_domain::code_mode::CodeCellObservation {cell:owner.clone(),
                after_output_sequence:0,wait_ms:0,terminate:false},
            &mut |response| {responses.push(response);Ok(())}).unwrap_err();
        assert!(error.contains("worker unavailable"));
        request.resume_after_sequence = Some(1);
        let error = driver.start_code_cell(&request,owner,Default::default(),0,
            "text('must not replay');".into(),vec![],
            &mut |response| {responses.push(response);Ok(())}).unwrap_err();
        assert!(error.contains("source cannot be replayed"));
        assert!(driver.code_cells.is_empty());
        assert!(responses.is_empty(),"lost worker cannot fabricate an output or successful completion");
    }
}

impl HostCodeCell {
    fn validate_control(&self, control: &HostRequestEnvelope) -> Result<(), String> {
        let parent = &self.request;
        if control.host_id != parent.host_id || control.session_id != parent.session_id
            || control.turn_id != parent.turn_id || control.lease_id != parent.lease_id {
            return Err("code cell control owner mismatch".into());
        }
        match (&parent.delivery, &control.delivery) {
            (Some(parent), Some(control)) if parent.worker_generation == control.worker_generation
                && parent.lease_owner == control.lease_owner => Ok(()),
            (None, None) => Ok(()),
            _ => Err("code cell control worker generation mismatch".into()),
        }
    }
}

impl CliAgentHostDriver {
    pub fn with_ui_publisher(mut self, publisher: std::sync::Arc<dyn Fn(nola_application::generative_ui::ExecuteUiTool) -> Result<serde_json::Value, nola_service_client::ApplicationServiceCommandError> + Send + Sync>) -> Self {
        self.ui_publisher = Some(publisher);
        let descriptor = nola_core::agent::generative_ui::capability_descriptor();
        self.catalog.capabilities.insert(descriptor.id.clone(), descriptor);
        self.capability_catalog.builtin_tools = build_cli_capability_catalog(&self.catalog).builtin_tools;
        self
    }

    pub fn code_mode_discoverable(&mut self) -> bool {
        self.refresh_current_turn_available_tool_ids();
        ["tool_search", "code_mode_exec", "code_mode_wait"].iter()
            .all(|id| self.current_turn_available_tool_ids.contains(*id))
    }

    pub fn with_code_mode_limits(mut self, limits: nola_domain::code_mode::CodeCellLimits) -> Result<Self, String> {
        limits.validate()?;
        if !self.code_cells.is_empty() { return Err("cannot change budgets of admitted code cells".into()); }
        for descriptor in nola_core::agent::code_mode::capability_descriptors(&limits)? {
            self.catalog.capabilities.insert(descriptor.id.clone(), descriptor);
        }
        // Code Mode owns built-in descriptors, not the separately discovered
        // MCP registry. Builder order must not erase connected providers.
        self.capability_catalog.builtin_tools =
            build_cli_capability_catalog(&self.catalog).builtin_tools;
        self.capability_catalog.revision = self.capability_catalog.revision.max(self.catalog.revision);
        self.code_mode_limits = limits;
        Ok(self)
    }

    fn stop_code_cell(&mut self, control: &HostRequestEnvelope, target: &str)
        -> Result<Option<ObservedHostStop>, String>
    {
        // Cancelling an observation only withdraws that observer. It does not
        // grant authority to stop the computation or sibling observations.
        for cell in self.code_cells.values_mut() {
            if let Some(index) = cell.observations.iter().position(|observer|
                observer.request.effect_id.0 == target) {
                cell.validate_control(control)?;
                let observer = cell.observations.remove(index);
                return Ok(Some(ObservedHostStop::from_request(&observer.request, None)));
            }
        }
        let Some(cell) = self.code_cells.values_mut().find(|cell| cell.request.effect_id.0 == target)
            else { return Ok(None); };
        cell.validate_control(control)?;
        cell.worker.request_stop()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if cell.worker.join_if_finished()? {
                cell.completed = true;
                return Ok(Some(ObservedHostStop::from_request(&cell.request, None)));
            }
            if std::time::Instant::now() >= deadline {
                return Err("code cell stop is not yet confirmed".into());
            }
            // Do not admit queued child calls after cancellation. Core owns
            // every previously committed child's independent stop receipt.
            let _ = cell.worker.next_event(std::time::Duration::from_millis(10));
        }
    }

    fn start_code_cell(&mut self, request: &HostRequestEnvelope,
        identity: nola_domain::code_mode::CodeCellIdentity,
        limits: nola_domain::code_mode::CodeCellLimits, initial_wait_ms: u32, source: String,
        tools: Vec<nola_domain::code_mode::CodeToolDescription>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>
    {
        limits.validate_within(&self.code_mode_limits)?;
        if initial_wait_ms > 60_000 { return Err("code cell initial observation wait exceeded".into()); }
        if identity.host_id != request.host_id.0 || identity.conversation_id != request.session_id
            || request.turn_id.as_deref() != Some(identity.turn_id.as_str()) {
            return Err("code cell identity differs from admitted delivery".into());
        }
        if self.code_cells.contains_key(&identity.cell_id) || request.resume_after_sequence.is_some() {
            return Err("code cell source cannot be replayed".into());
        }
        let id = identity.cell_id.clone();
        let worker = nola_code_mode_host::worker::CellWorker::start(identity, limits, source, tools)?;
        self.code_cells.insert(id.clone(), HostCodeCell { request: request.clone(), worker,
            sequence: Self::response_sequence(request), output: Vec::new(), completed: false,
            awaiting: Default::default(), observations: Vec::new() });
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(u64::from(initial_wait_ms));
        self.drain_code_cell_until(&id, Some(deadline), false, emit)?;
        let cell = self.code_cells.get_mut(&id).ok_or("code cell worker unavailable")?;
        if !cell.completed {
            if let Err(error) = Self::emit(&cell.request, &mut cell.sequence, HostResponse::CodeModeYielded {
                cell:cell.worker.owner().clone(), output_cursor:cell.output.len() as u64,
            }, emit) {
                let _ = cell.worker.request_stop();
                return Err(error);
            }
        }
        Ok(())
    }

    fn drain_code_cell(&mut self, id: &str,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>
    {
        // Internal result delivery/poll is not a model-requested wait. Keep
        // it below its 30s control deadline even when JS keeps computing.
        // The live cell and its output cursor remain available for observation.
        self.drain_code_cell_until(id,
            Some(std::time::Instant::now() + std::time::Duration::from_secs(1)), false, emit)
    }

    fn observe_code_cell(&mut self, request: &HostRequestEnvelope, call_id: String,
        observation: nola_domain::code_mode::CodeCellObservation,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>
    {
        let cell = self.code_cells.get(&observation.cell.cell_id)
            .ok_or("code cell worker unavailable; source replay is forbidden")?;
        cell.validate_control(request)?;
        observation.validate(cell.worker.owner())?;
        if observation.terminate { return Err("code cell cancellation must be dispatched through Core cancellation".into()); }
        let previous = cell.observations.iter().find(|observer| observer.request.effect_id == request.effect_id);
        if previous.is_some_and(|observer| observer.call_id != call_id
            || observer.cursor != observation.after_output_sequence) {
            return Err("code observation replay changed its call or cursor".into());
        }
        let deadline = previous.map(|observer| observer.deadline).unwrap_or_else(||
            std::time::Instant::now() + std::time::Duration::from_millis(u64::from(observation.wait_ms)));
        let mut sequence = previous.map_or(Self::response_sequence(request), |observer|
            observer.sequence.max(Self::response_sequence(request)));
        self.drain_code_cell_until(&observation.cell.cell_id, Some(deadline), true, emit)?;
        let cell = self.code_cells.get_mut(&observation.cell.cell_id).ok_or("code cell worker unavailable")?;
        if !cell.completed && cell.output.len() as u64 <= observation.after_output_sequence
            && std::time::Instant::now() < deadline {
            Self::emit(request, &mut sequence, HostResponse::CodeModeObservationSuspended {
                cell: observation.cell.clone(), call_id: call_id.clone(),
            }, emit)?;
            cell.observations.retain(|observer| observer.request.effect_id != request.effect_id);
            cell.observations.push(HostCodeObservation { request: request.clone(), call_id,
                cursor: observation.after_output_sequence, deadline, sequence });
            return Ok(());
        }
        Self::emit(request, &mut sequence, HostResponse::ToolCompleted {
            call_id, output: serde_json::json!({"cell_id":observation.cell.cell_id}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None, full_output_ref: None, artifacts: Vec::new(),
        }, emit)?;
        cell.observations.retain(|observer| observer.request.effect_id != request.effect_id);
        Ok(())
    }

    fn settle_code_observations(&mut self, id: &str,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>
    {
        let cell = self.code_cells.get_mut(id).ok_or("code cell worker unavailable")?;
        let mut index = 0;
        while index < cell.observations.len() {
            let observer = &mut cell.observations[index];
            if !cell.completed && cell.output.len() as u64 <= observer.cursor
                && std::time::Instant::now() < observer.deadline {
                index += 1;
                continue;
            }
            Self::emit(&observer.request, &mut observer.sequence, HostResponse::ToolCompleted {
                call_id: observer.call_id.clone(), output: serde_json::json!({"cell_id":id}),
                outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                activation: None, full_output_ref: None, artifacts: Vec::new(),
            }, emit)?;
            cell.observations.remove(index);
        }
        Ok(())
    }

    fn drain_code_cell_until(&mut self, id: &str, deadline: Option<std::time::Instant>,
        return_after_output: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>
    {
        use nola_code_mode_host::worker::CellEvent;
        let cell = self.code_cells.get_mut(id).ok_or("code cell worker unavailable")?;
        if cell.completed { return Ok(()); }
        let mut yielded = false;
        loop {
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) { return Ok(()); }
            if self.tool_context.cancellation.is_requested() {
                cell.worker.request_stop()?;
            }
            // After enqueueing nested work, flush the current callback burst
            // and yield so the same Host can dispatch Core's child requests.
            let mut timeout = if yielded { std::time::Duration::from_millis(1) }
                else { std::time::Duration::from_millis(100) };
            if let Some(deadline) = deadline { timeout = timeout.min(deadline.saturating_duration_since(std::time::Instant::now())); }
            let Some(event) = cell.worker.next_event(timeout)? else {
                if yielded || !cell.awaiting.is_empty() { return Ok(()); }
                continue;
            };
            match event {
                CellEvent::ToolRequested(request) => {
                    let nested_sequence = request.sequence;
                    if let Err(error) = Self::emit(&cell.request, &mut cell.sequence,
                        HostResponse::CodeModeToolRequested { request }, emit) {
                        let _ = cell.worker.request_stop();
                        return Err(error);
                    }
                    cell.awaiting.insert(nested_sequence);
                    yielded = true;
                },
                CellEvent::Output(text) => {
                    cell.output.push(text.clone());
                    let frame = nola_domain::code_mode::CodeOutputFrame {
                        cell: cell.worker.owner().clone(),
                        chunk: nola_domain::code_mode::CodeOutputChunk {
                            sequence: cell.output.len() as u64, text,
                        },
                    };
                    if let Err(error) = Self::emit(&cell.request, &mut cell.sequence,
                        HostResponse::CodeModeOutput { frame }, emit) {
                        let _ = cell.worker.request_stop();
                        return Err(error);
                    }
                    if return_after_output { return Ok(()); }
                },
                CellEvent::Finished(result) => {
                    let (mut outcome, mut error) = match result {
                        Ok(()) => (nola_core::agent::host::HostToolOutcome::Succeeded, None),
                        Err(error) if self.tool_context.cancellation.is_requested() =>
                            (nola_core::agent::host::HostToolOutcome::Cancelled, Some(error)),
                        Err(error) => (nola_core::agent::host::HostToolOutcome::Failed, Some(error)),
                    };
                    let full_output_ref = if cell.output.is_empty() { None } else {
                        match self.tool_context.output_store.persist(&cell.output.join("\n")) {
                            Ok(path) => Some(path.to_string_lossy().into_owned()),
                            Err(storage_error) => {
                                // The engine already ended. Settle a failure;
                                // do not leave a phantom running worker or retry
                                // its source to recover a failed archive write.
                                outcome = nola_core::agent::host::HostToolOutcome::Failed;
                                error = Some(format!("code cell output archive failed: {storage_error}; engine: {error:?}"));
                                None
                            },
                        }
                    };
                    Self::emit(&cell.request, &mut cell.sequence, HostResponse::ToolCompleted {
                        call_id: cell.worker.owner().parent_call_id.clone(),
                        // Output was already acknowledged through ordered
                        // CodeModeOutput frames. Do not serialize the whole
                        // stream again in the terminal receipt/checkpoint.
                        output: serde_json::json!({"cell_id":id,"error":error}),
                        outcome, activation: None, full_output_ref, artifacts: Vec::new(),
                    }, emit)?;
                    cell.completed = true;
                    cell.output = Vec::new();
                    let _ = cell.worker.join_if_finished()?;
                    return Ok(());
                },
            }
        }
    }
}
