// SPDX-License-Identifier: Apache-2.0
//! Embedded child transport. The Application owns all state and the shared
//! committed driver owns all pumping; native code only executes polled requests.
use super::{encode, utf8_input, EmbeddedApplication, NolaCoreApplication};
use nola_core::agent::host::{
    AgentBoundaryInput, AgentRuntimeProtocol, EffectDeliveryLease, HostError, HostRequest,
    HostRequestEnvelope, HostResponse, HostResponseEnvelope,
};
use nola_domain::task_execution::{RuntimeTaskAttempt, RuntimeTaskRunBinding};
use nola_runtime_driver::mailbox::{bounded_host_mailbox, HostMailbox};
use nola_runtime_driver::{
    CommittedRuntimeAcceptance, CommittedRuntimeApplicationPort, CommittedRuntimeDriver,
    RuntimeEffectDeliveryClaim, RuntimeEffectDeliveryRenewal, RuntimeEffectDeliveryTiming,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    os::raw::c_char,
    sync::{Arc, Mutex},
    time::Duration,
};

type Acceptance = CommittedRuntimeAcceptance<AgentRuntimeProtocol>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    task_id: String,
    parent_conversation_id: String,
    conversation_id: String,
    run_id: String,
    assistant_message_id: String,
    attempt: RuntimeTaskAttempt,
    cancellation_only: bool,
    worker: nola_host_protocol::HostWorkerIdentity,
    current: nola_application::RunRecoveryAuthority,
    mailbox_capacity: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    pump_id: String,
    ticket: u64,
    response: HostResponseEnvelope,
}

pub struct NolaCoreChildPump {
    id: String,
    mailbox: HostMailbox<AgentRuntimeProtocol>,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
}
struct Port {
    app: Arc<EmbeddedApplication>,
    conversation: String,
    run: String,
    assistant: String,
    digest: String,
    host_invocation: nola_host_protocol::HostInvocationBinding,
    attempt: RuntimeTaskAttempt,
    cancellation_only: bool,
    task_lease_ms: i64,
    mailbox: HostMailbox<AgentRuntimeProtocol>,
}
fn command<T: DeserializeOwned>(app: &Arc<EmbeddedApplication>, value: Value) -> Result<T, String> {
    let command = serde_json::from_value(value)
        .map_err(|error| format!("invalid child Application command: {error}"))?;
    let value = app
        .lock()
        .map_err(|_| "Application lock poisoned")?
        .execute(command)?;
    serde_json::from_value(value)
        .map_err(|error| format!("invalid child Application receipt: {error}"))
}
fn now() -> i64 {
    nola_runtime_driver::monotonic_epoch_millis()
}
impl Port {
    fn recover_deliveries(&mut self) -> Result<(), String> {
        match self.recover_deliveries_inner() {
            Ok(()) => Ok(()),
            Err(error) => {
                // Another legitimate pump may seal after the first active
                // read. Only that exact frozen terminal owner absorbs a race.
                let current: Option<nola_application::ExactRunInvocation> = self.task_command(Some(json!({
                    "type": "load_exact_run_invocation", "conversation_id": self.conversation,
                    "expected_run_id": self.run, "assistant_message_id": self.assistant,
                })))?;
                if current.is_some_and(|value| value.status == "sealed"
                    && value.invocation_config_digest == self.digest && value.host_invocation == self.host_invocation) {
                    Ok(())
                } else { Err(error) }
            }
        }
    }
    fn recover_deliveries_inner(&mut self) -> Result<(), String> {
        let exact: Option<nola_application::ExactRunInvocation> = self.task_command(Some(json!({
            "type": "load_exact_run_invocation",
            "conversation_id": self.conversation,
            "expected_run_id": self.run,
            "assistant_message_id": self.assistant,
        })))?;
        let exact = exact.ok_or("child recovery invocation missing")?;
        if exact.status == "sealed" {
            return Ok(());
        }
        let time = now();
        let interrupted_at = chrono::DateTime::from_timestamp_millis(time)
            .ok_or("invalid recovery time")?
            .to_rfc3339();
        let plan: nola_application::run_process_observation::RunProcessObservationPlan = self.task_command(Some(json!({
            "type": "inspect_run_process_owners", "conversation_id": self.conversation, "expected_run_id": self.run,
        })))?;
        let mut observed_recovered = false;
        if !plan.has_session_lease && !plan.deliveries.is_empty() {
            let observations: Vec<_> = plan.process_owners.iter().map(|owner|
                nola_application::run_process_observation::RunProcessObservation {
                    owner: owner.clone(), observation: crate::process_observation::observe(owner.pid),
                }).collect();
            let terminated: bool = self.task_command(Some(json!({
                "type": "validate_run_process_observations", "plan": plan, "observations": observations,
            })))?;
            if terminated {
                let _: Value = self.task_command(Some(json!({
                    "type": "recover_observed_orphaned_run", "conversation_id": self.conversation,
                    "expected_run_id": self.run, "host_invocation": self.host_invocation,
                    "observed_session_lease": null, "observed_deliveries": plan.deliveries,
                    "interrupted_at": interrupted_at, "interrupted_at_ms": time,
                })))?;
                observed_recovered = true;
            }
        }
        if !observed_recovered {
            let _: Value = self.task_command(Some(json!({
                "type": "recover_expired_run_deliveries",
                "conversation_id": self.conversation,
                "expected_run_id": self.run,
                "host_invocation": self.host_invocation,
                "interrupted_at": interrupted_at,
                "interrupted_at_ms": time,
            })))?;
        }
        // Reconciliation may have sealed the Run. Never reconcile it a second
        // time or rebound effects against a changed canonical owner.
        let refreshed: Option<nola_application::ExactRunInvocation> = self.task_command(Some(json!({
            "type": "load_exact_run_invocation", "conversation_id": self.conversation,
            "expected_run_id": self.run, "assistant_message_id": self.assistant,
        })))?;
        let refreshed = refreshed.ok_or("child recovery lost its exact invocation")?;
        if refreshed.status == "sealed" { return Ok(()); }
        if refreshed.invocation_config_digest != self.digest || refreshed.host_invocation != self.host_invocation {
            return Err("child recovery invocation changed".into());
        }
        let _: Value = self.task_command(Some(json!({
            "type": "prepare_run_effect_recovery",
            "input": {
                "conversation_id": self.conversation,
                "run_id": self.run,
                "invocation_config_digest": self.digest,
                "now_ms": time,
            },
        })))?;
        Ok(())
    }
    fn task_command<T: DeserializeOwned>(&mut self, operation: Option<Value>) -> Result<T, String> {
        let mut authority_validated = false;
        let result = (|| {
            // Keep the authority read, optional renewal, and operation in the
            // SAME embedded Application serialization scope. No foreground
            // mutable Run or split check/use window participates.
            let mut app = self.app.lock().map_err(|_| "Application lock poisoned")?;
            let mut execute = |value: Value| -> Result<Value, String> {
                app.execute(
                    serde_json::from_value(value)
                        .map_err(|error| format!("child command: {error}"))?,
                )
            };
            let current: Option<RuntimeTaskAttempt> =
                serde_json::from_value(execute(json!({"type":"runtime_task",
                "command":{"type":"current_attempt","task_id":self.attempt.task_id}}))?)
                .map_err(|error| error.to_string())?;
            if self.cancellation_only {
                let task: nola_application::RuntimeTask = serde_json::from_value(execute(json!({
                    "type":"runtime_task","command":{"type":"get","id":self.attempt.task_id}}))?)
                    .map_err(|error| error.to_string())?;
                let binding: Option<RuntimeTaskRunBinding> = serde_json::from_value(execute(json!({
                    "type":"runtime_task","command":{"type":"run_binding","task_id":self.attempt.task_id}}))?)
                    .map_err(|error| error.to_string())?;
                if !task.cancel_requested || current.as_ref() != Some(&self.attempt)
                    || self.attempt.lease_expires_at_ms > now()
                    || !binding.is_some_and(|b| b.run_id == self.run && b.child_session_id == self.conversation) {
                    return Err("cancel-only child authority changed".into());
                }
                authority_validated = true;
                return serde_json::from_value(match operation { Some(value) => execute(value)?, None => Value::Null })
                    .map_err(|error| error.to_string());
            }
            if current.as_ref() != Some(&self.attempt)
                || self.attempt.state != "leased"
                || self.attempt.lease_expires_at_ms <= now()
            {
                return Err("child Task attempt lost its exact live lease".into());
            }
            if self.attempt.lease_expires_at_ms - now() <= self.task_lease_ms / 2 {
                let time = now();
                let expiry = time.saturating_add(self.task_lease_ms);
                let next: RuntimeTaskAttempt = serde_json::from_value(execute(json!({"type":"runtime_task","command":{
                    "type":"renew_lease","task_id":self.attempt.task_id,"attempt_id":self.attempt.id,
                    "expected_attempt_revision":self.attempt.revision,"worker_generation":self.attempt.worker_generation,
                    "lease_token":self.attempt.lease_token,"current_expiry_ms":self.attempt.lease_expires_at_ms,
                    "now_ms":time,"requested_expiry_ms":expiry}}))?).map_err(|error|error.to_string())?;
                nola_runtime_driver::task_execution_authority::accept_runtime_task_attempt(
                    &mut self.attempt,
                    &next,
                    expiry,
                )?;
            }
            authority_validated = true;
            serde_json::from_value(match operation {
                Some(value) => execute(value)?,
                None => Value::Null,
            })
            .map_err(|error| error.to_string())
        })();
        if result.is_err() && !authority_validated {
            self.mailbox.close();
        }
        result
    }
    fn task_live(&mut self) -> Result<(), String> {
        self.task_command::<Value>(None)?;
        // Task controls are durable Application input, not messages supplied by
        // the native mailbox. Observe them while a physical model is in flight
        // through the driver's existing heartbeat/control cadence.
        let task: nola_application::RuntimeTask = self
            .task_command(Some(json!({"type":"runtime_task",
            "command":{"type":"get","id":self.attempt.task_id}})))?;
        if task.cancel_requested {
            let requested_at = task
                .cancel_requested_at_ms
                .ok_or("Task cancellation has no committed timestamp")?;
            self.task_command::<Value>(Some(json!({"type":"request_run_cancellation","input":{
                "conversation_id":self.conversation,"expected_run_id":self.run,"assistant_message_id":self.assistant,
                "cancellation_id":format!("task-cancel:{}:{}",self.attempt.task_id,self.run),
                "reason":"task cancellation requested","requested_at_ms":requested_at}})))?;
        } else {
            let pending: Vec<nola_application::RuntimeTaskGuidance> = self.task_command(Some(json!({"type":"runtime_task",
                "command":{"type":"pending_task_guidance","task_id":self.attempt.task_id,"expected_run_id":self.run}})))?;
            for guidance in pending {
                self.task_command::<Value>(Some(json!({"type":"apply_task_guidance","task_id":self.attempt.task_id,
                    "message_id":guidance.message_id,"lease":nola_domain::task_execution::RuntimeTaskLeaseWitness::from(&self.attempt),
                    "expected_run_id":self.run,"now_ms":now()})))?;
            }
        }
        Ok(())
    }
    fn requests(
        &mut self,
        kind: &str,
        claim: Option<RuntimeEffectDeliveryClaim>,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        let input = match claim {
            Some(claim) => {
                json!({"type":kind,"input":{"run_id":self.run,"worker_generation":claim.worker_generation,
                "lease_owner":claim.lease_owner,"claimed_at_ms":claim.claimed_at_ms,
                "lease_expires_at_ms":claim.lease_expires_at_ms,"limit":claim.limit}})
            }
            None => json!({"type":kind,"run_id":self.run}),
        };
        let result: Value = if kind == "load_committed_host_requests" {
            command(&self.app, input)?
        } else {
            self.task_command(Some(input))?
        };
        let requests: Vec<HostRequestEnvelope> = serde_json::from_value(
            result
                .get("requests")
                .cloned()
                .ok_or("missing committed child requests")?,
        )
        .map_err(|error| error.to_string())?;
        if self.cancellation_only && requests.iter().any(|request| !matches!(request.payload, HostRequest::Cancel { .. })) {
            return Err("cancel-only child cannot execute business effects".into());
        }
        Ok(requests)
    }
}
impl CommittedRuntimeApplicationPort for Port {
    type Protocol = AgentRuntimeProtocol;
    fn dispatch_application_request(
        &mut self,
        request: &HostRequestEnvelope,
        publish: &mut dyn FnMut(Acceptance),
    ) -> Result<bool, String> {
        if !matches!(
            request.payload,
            HostRequest::CancelTask { .. } | HostRequest::ReadCodeCellHistory { .. }
        ) {
            return Ok(false);
        }
        let mut sequence = request
            .resume_after_sequence
            .map_or(0, |sequence| sequence.saturating_add(1));
        let mut response = |payload| {
            let envelope = HostResponseEnvelope {
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
            };
            sequence = sequence.saturating_add(1);
            envelope
        };
        if request.is_expired_at(now() as u64) {
            publish(
                self.accept_raw_host_response(response(HostResponse::ExecutionRejected {
                    code: "deadline_exceeded".into(),
                    message: "child Application request expired before dispatch".into(),
                    retryable: false,
                }))?,
            );
            return Ok(true);
        }
        if request.delivery.is_some() && !request.attempt_started {
            publish(self.accept_raw_host_response(response(HostResponse::ExecutionStarted))?);
        }
        let payload = match &request.payload {
            HostRequest::CancelTask { task_id } => {
                let result:Result<bool,String>=self.task_command(Some(json!({"type":"execute_task_cancellation_for_effect","request":request,"now_ms":now()})));
                match result {
                    Ok(requested) => HostResponse::Completed {
                        output: json!({"task_id":task_id,"cancel_requested":requested}),
                        usage: None,
                        full_output_ref: None,
                        artifacts: vec![],
                    },
                    Err(error) => HostResponse::Failed {
                        error: HostError {
                            code: "task_cancellation_failed".into(),
                            message: error,
                            retryable: false,
                            http_status: None,
                        },
                    },
                }
            }
            HostRequest::ReadCodeCellHistory { call_id, .. } => {
                let result: Result<Value, String> = self.task_command(Some(
                    json!({"type":"read_code_cell_history_for_effect","request":request}),
                ));
                let (output, outcome) = match result {
                    Ok(value) if !value.is_null() => {
                        (value, nola_domain::host_effect::HostToolOutcome::Succeeded)
                    }
                    Ok(_) => (
                        json!({"error":"code cell history not found"}),
                        nola_domain::host_effect::HostToolOutcome::Failed,
                    ),
                    Err(error) => (
                        json!({"error":error}),
                        nola_domain::host_effect::HostToolOutcome::Failed,
                    ),
                };
                HostResponse::ToolCompleted {
                    call_id: call_id.clone(),
                    output,
                    outcome,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![],
                }
            }
            _ => unreachable!(),
        };
        publish(self.accept_raw_host_response(response(payload))?);
        Ok(true)
    }
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        self.requests("load_committed_host_requests", None)
    }
    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        self.task_live()?;
        self.recover_deliveries()?;
        self.requests("claim_committed_host_requests", Some(claim))
    }
    fn claim_application_control_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
        _observed: &[HostRequestEnvelope],
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        self.task_live()?;
        self.requests("claim_application_control_requests", Some(claim))
    }
    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String> {
        self.task_live()?;
        let lease = renewal.delivery;
        self.task_command(Some(json!({"type":"heartbeat_effect_delivery","input":{"run_id":self.run,
            "attempt_id":lease.attempt_id,"worker_generation":lease.worker_generation,"lease_owner":lease.lease_owner,
            "lease_token":lease.lease_token,"current_lease_expires_at_ms":lease.lease_expires_at_ms,
            "heartbeat_at_ms":renewal.renewed_at_ms,"requested_lease_expires_at_ms":renewal.lease_expires_at_ms}})))
    }
    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<Acceptance, String> {
        self.task_live()?;
        let result: nola_application::AcceptedHostResponse = self.task_command(Some(json!({"type":"reduce_and_accept_host_response",
            "input":{"conversation_id":self.conversation,"run_id":self.run,"invocation_config_digest":self.digest,
                "response":response,"received_at_ms":now()}})))?;
        Ok(Acceptance {
            events: result.events,
            traces: result.traces,
            control: result.control,
        })
    }
    fn accept_boundary_input(&mut self, _input: AgentBoundaryInput) -> Result<Acceptance, String> {
        Err("child mailbox has no unauthenticated boundary input channel; submit through Application".into())
    }
}
fn parse<T: DeserializeOwned>(data: *const u8, len: usize) -> Result<T, String> {
    if len > nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES {
        return Err("child mailbox frame exceeds limit".into());
    }
    let value = utf8_input(data, len).ok_or("invalid child mailbox UTF-8")?;
    serde_json::from_str(&value).map_err(|error| error.to_string())
}
fn failure(message: impl ToString) -> *mut c_char {
    encode(
        json!({"ok":false,"error":{"code":"child_transport_unconfirmed","message":message.to_string()}}),
    )
}

/// error_json is either null or an allocated error freed with nola_core_free_c_string.
#[no_mangle]
pub extern "C" fn nola_core_child_pump_start(
    application: *mut NolaCoreApplication,
    data: *const u8,
    len: usize,
    error_json: *mut *mut c_char,
) -> *mut NolaCoreChildPump {
    if !error_json.is_null() {
        unsafe {
            *error_json = std::ptr::null_mut();
        }
    }
    let result: Result<*mut NolaCoreChildPump, String> = (|| {
        if application.is_null() {
            return Err("Application unavailable".into());
        }
        let start: Start = parse(data, len)?;
        let app = unsafe { &*application }.application.clone();
        let binding: Option<RuntimeTaskRunBinding> = command(
            &app,
            json!({"type":"runtime_task","command":{"type":"run_binding","task_id":start.task_id}}),
        )?;
        let binding = binding.ok_or("child requires an already admitted exact Task Run binding")?;
        if binding.task_id != start.task_id
            || binding.owner_session_id != start.parent_conversation_id
            || binding.child_session_id != start.conversation_id
            || binding.run_id != start.run_id
            || start.attempt.task_id != start.task_id
        {
            return Err("child pump identity differs from immutable Task Run binding".into());
        }
        let exact: Option<nola_application::ExactRunInvocation> = command(
            &app,
            json!({"type":"load_exact_run_invocation",
            "conversation_id":start.conversation_id,"expected_run_id":start.run_id,"assistant_message_id":start.assistant_message_id}),
        )?;
        let exact = exact.ok_or("child Run invocation missing")?;
        if !start.cancellation_only { let _: Value = command(
            &app,
            json!({"type":"validate_run_recovery_authority","conversation_id":start.conversation_id,
            "expected_run_id":start.run_id,"assistant_message_id":start.assistant_message_id,
            "expected_invocation_config_digest":exact.invocation_config_digest,"current":start.current}),
        )?; }
        let identity = start.worker.lease_identity()?;
        if !start.cancellation_only && identity.worker_generation != start.attempt.worker_generation {
            return Err("child Task lease belongs to another worker generation".into());
        }
        let (host, mailbox) = bounded_host_mailbox::<AgentRuntimeProtocol>(start.mailbox_capacity)?;
        let task_lease_ms = if start.cancellation_only { 30_000 } else { start
            .attempt
            .lease_expires_at_ms
            .checked_sub(now())
            .filter(|duration| (1000..=600_000).contains(duration))
            .ok_or("invalid child Task lease duration")? };
        let mut port = Port {
            app: app.clone(),
            conversation: start.conversation_id,
            run: start.run_id,
            assistant: start.assistant_message_id,
            digest: exact.invocation_config_digest,
            host_invocation: start.current.host_invocation,
            attempt: start.attempt,
            cancellation_only: start.cancellation_only,
            task_lease_ms,
            mailbox: mailbox.clone(),
        };
        port.task_live()?;
        port.recover_deliveries()?;
        let outcome = Arc::new(Mutex::new(None));
        let status = outcome.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let observer = mailbox.clone();
        // Renewal cadence stays comfortably inside both independent leases.
        let cadence = (task_lease_ms / 4).clamp(100, 1000) as u64;
        let abort_mailbox = mailbox.clone();
        let mut driver = CommittedRuntimeDriver::with_delivery_identity_and_timing(
            host,
            identity.worker_generation,
            identity.lease_owner,
            RuntimeEffectDeliveryTiming {
                lease_duration: Duration::from_secs(30),
                heartbeat_cadence: Duration::from_millis(cadence),
            },
        ).with_delivery_abort(Arc::new(move |effect_id| {
            // Transport abort only. The shared driver then drains the exact
            // committed Cancel, whose native handler must prove physical stop.
            let _ = abort_mailbox.abort_delivery(effect_id);
        }));
        std::thread::Builder::new().name(format!("nola-child-{}",start.task_id)).spawn(move||{
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                driver.execute(&mut port,&mut |_|{}).map_err(|error|format!("child pump: {error:?}"))))
                .unwrap_or_else(|_|Err("child pump panicked; execution outcome remains unconfirmed".into()));
            // This can settle only a committed child seal, never transport completion.
            let result=result.and_then(|_| command::<Value>(&app,json!({"type":"runtime_task","command":{
                "type":"reconcile_bound_run_terminal","task_id":port.attempt.task_id,"expected_run_id":port.run,"now_ms":now()}})).map(|_|()));
            if let Ok(mut state)=status.lock(){*state=Some(result);}
            observer.close();
        }).map_err(|error|error.to_string())?;
        Ok(Box::into_raw(Box::new(NolaCoreChildPump {
            id,
            mailbox,
            outcome,
        })))
    })();
    match result {
        Ok(handle) => handle,
        Err(message) => {
            if !error_json.is_null() {
                unsafe {
                    *error_json = failure(message);
                }
            }
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "C" fn nola_core_child_pump_poll(
    pump: *mut NolaCoreChildPump,
    wait_ms: u32,
) -> *mut c_char {
    if pump.is_null() {
        return failure("child pump unavailable");
    }
    let pump = unsafe { &*pump };
    if let Ok(state) = pump.outcome.lock() {
        if let Some(outcome) = state.as_ref() {
            return encode(
                json!({"ok":true,"result":{"pump_id":pump.id,"transport_finished":true,"error":outcome.as_ref().err()}}),
            );
        }
    }
    match pump.mailbox.poll(Duration::from_millis(wait_ms as u64)) {
        Ok(request) => {
            let value = json!({"ok":true,"result":{"pump_id":pump.id,"transport_finished":false,
                "request":request.map(|(ticket,request)|json!({"ticket":ticket,"envelope":request}))}});
            if serde_json::to_vec(&value).map_or(true, |bytes| {
                bytes.len() > nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES
            }) {
                pump.mailbox.close();
                return failure("child HostRequest exceeds Application frame limit");
            }
            encode(value)
        }
        Err(error) => {
            if let Ok(state) = pump.outcome.lock() {
                if let Some(outcome) = state.as_ref() {
                    return encode(json!({"ok":true,"result":{
                "pump_id":pump.id,"transport_finished":true,"error":outcome.as_ref().err()}}));
                }
            }
            failure(error)
        }
    }
}
#[no_mangle]
pub extern "C" fn nola_core_child_pump_respond(
    pump: *mut NolaCoreChildPump,
    data: *const u8,
    len: usize,
) -> *mut c_char {
    if pump.is_null() {
        return failure("child pump unavailable");
    }
    let pump = unsafe { &*pump };
    let result: Result<(), String> = (|| {
        let reply: Reply = parse(data, len)?;
        if reply.pump_id != pump.id {
            return Err("foreign child pump ticket".into());
        }
        pump.mailbox.respond(reply.ticket, reply.response)
    })();
    match result {
        Ok(()) => encode(json!({"ok":true,"result":{"committed":true}})),
        Err(error) => failure(error),
    }
}
/// Safe to call concurrently with poll/respond to unblock them. This does not free the handle.
#[no_mangle]
pub extern "C" fn nola_core_child_pump_close(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        unsafe { &*pump }.mailbox.close();
    }
}
/// Close first, then wait for every concurrent FFI call to return before free.
#[no_mangle]
pub extern "C" fn nola_core_child_pump_free(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        let pump = unsafe { Box::from_raw(pump) };
        pump.mailbox.close();
    }
}
