// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Drives the functional Agent Core through a platform Host effect executor.

#![allow(dead_code)]

pub mod task_execution_authority;
pub mod task_worker;
pub mod mailbox;

use nola_host_protocol::{
    BoundaryInputSource, EffectDeliveryLease, HostDriver, HostExecutionProtocol,
    HostRequestEnvelope, HostResponseEnvelope, RuntimeRequestPayload, RuntimeResponsePayload,
};
use nola_domain::core_event::CoreEventEnvelope;

type Request<P> = HostRequestEnvelope<<P as HostExecutionProtocol>::Request>;
type Response<P> = HostResponseEnvelope<<P as HostExecutionProtocol>::Response>;

#[derive(Debug, Clone)]
pub enum CoreHostOutput<P: HostExecutionProtocol> {
    /// An acceptance committed; readers may drain its journal publications.
    Committed,
    Event(CoreEventEnvelope<P::Event>),
    Trace(P::Trace),
    Control(P::Control),
}

#[derive(Debug)]
pub enum RuntimeDriverError<E> {
    Core(String),
    Host(E),
    ProtocolStalled { effect_ids: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct CommittedRuntimeAcceptance<P: HostExecutionProtocol> {
    pub events: Vec<CoreEventEnvelope<P::Event>>,
    pub traces: Vec<P::Trace>,
    pub control: Option<P::Control>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectDeliveryClaim {
    pub worker_generation: String,
    pub lease_owner: String,
    pub claimed_at_ms: i64,
    pub lease_expires_at_ms: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectDeliveryRenewal {
    pub delivery: EffectDeliveryLease,
    pub renewed_at_ms: i64,
    pub lease_expires_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeEffectDeliveryTiming {
    pub lease_duration: std::time::Duration,
    pub heartbeat_cadence: std::time::Duration,
}

impl Default for RuntimeEffectDeliveryTiming {
    fn default() -> Self {
        Self {
            lease_duration: std::time::Duration::from_millis(300_000),
            heartbeat_cadence: std::time::Duration::from_millis(100_000),
        }
    }
}

/// Transaction boundary for the committed-only Runtime Driver.
/// Application, not Runtime, restores and reduces Core state.
pub trait CommittedRuntimeApplicationPort {
    type Protocol: HostExecutionProtocol;

    fn load_committed_requests(&mut self) -> Result<Vec<Request<Self::Protocol>>, String>;

    /// Resolve Application-owned observations through this exact transaction
    /// owner. Return false only when the request belongs to a physical Host.
    /// Publish each accepted receipt before advancing to the next boundary.
    fn dispatch_application_request(
        &mut self,
        _request: &Request<Self::Protocol>,
        _publish: &mut dyn FnMut(CommittedRuntimeAcceptance<Self::Protocol>),
    ) -> Result<bool, String> {
        Ok(false)
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<Request<Self::Protocol>>, String>;

    /// Claim only Core-selected Application controls while a physical batch is
    /// in flight. The observed outbox is a wake hint, never claim authority.
    fn claim_application_control_requests(
        &mut self,
        _claim: RuntimeEffectDeliveryClaim,
        _observed: &[Request<Self::Protocol>],
    ) -> Result<Vec<Request<Self::Protocol>>, String> { Ok(Vec::new()) }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String>;

    fn accept_raw_host_response(
        &mut self,
        response: Response<Self::Protocol>,
    ) -> Result<CommittedRuntimeAcceptance<Self::Protocol>, String>;

    /// Every boundary input uses Application's canonical invocation and UoW.
    /// The pump cannot interpret submissions or import Task projections itself.
    fn accept_boundary_input(
        &mut self,
        input: <Self::Protocol as HostExecutionProtocol>::BoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance<Self::Protocol>, String>;
}

/// Side-effect pump with no reducer state and no uncommitted work queue.
pub struct CommittedRuntimeDriver<D: HostDriver> {
    driver: D,
    worker_generation: String,
    lease_owner: String,
    delivery_timing: RuntimeEffectDeliveryTiming,
    delivery_clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync>,
    delivery_abort: Option<std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
    /// Recoverable delegated effects deliberately yield while their separately
    /// committed permission request owns the interactive wait. Keep the exact
    /// parent fence across pump calls so resumed backend events can be admitted.
    parked_deliveries: std::collections::HashMap<String, ParkedDelivery>,
}

impl<D: HostDriver> CommittedRuntimeDriver<D> {
    pub fn new(driver: D, process: nola_host_protocol::HostWorkerProcessIdentity) -> Result<Self, String> {
        let lease = process.lease_identity()?;
        Ok(Self::with_delivery_identity(driver, lease.worker_generation, lease.lease_owner))
    }

    pub fn with_delivery_identity(
        driver: D,
        worker_generation: impl Into<String>,
        lease_owner: impl Into<String>,
    ) -> Self {
        Self {
            driver,
            worker_generation: worker_generation.into(),
            lease_owner: lease_owner.into(),
            delivery_timing: RuntimeEffectDeliveryTiming::default(),
            delivery_clock: std::sync::Arc::new(monotonic_epoch_millis),
            delivery_abort: None,
            parked_deliveries: Default::default(),
        }
    }

    pub fn with_delivery_identity_and_timing(
        driver: D,
        worker_generation: impl Into<String>,
        lease_owner: impl Into<String>,
        delivery_timing: RuntimeEffectDeliveryTiming,
    ) -> Self {
        validate_delivery_timing(delivery_timing);
        Self {
            driver,
            worker_generation: worker_generation.into(),
            lease_owner: lease_owner.into(),
            delivery_timing,
            delivery_clock: std::sync::Arc::new(monotonic_epoch_millis),
            delivery_abort: None,
            parked_deliveries: Default::default(),
        }
    }

    /// Supplies the epoch clock used for durable lease claims and renewals.
    /// The caller must supply nondecreasing epoch milliseconds in the same
    /// clock domain as Application. Heartbeat scheduling still uses Instant.
    pub fn with_delivery_clock(
        mut self,
        clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        self.delivery_clock = clock;
        self
    }

    /// Installs the cooperative abort edge used when Application rejects a
    /// lease renewal. The callback must fan out to all external work owned by
    /// this Host dispatch.
    pub fn with_delivery_abort(
        mut self,
        abort: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        self.delivery_abort = Some(abort);
        self
    }

    pub fn driver(&self) -> &D {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut D {
        &mut self.driver
    }

    /// Starts a fresh physical delivery namespace after Core explicitly
    /// rebinds an interrupted effect. The successor keeps the logical effect
    /// identity but must not reuse its predecessor's generation or owner.
    pub fn renew_delivery_identity(&mut self, process: nola_host_protocol::HostWorkerProcessIdentity) -> Result<(), String> {
        let lease = process.lease_identity()?;
        self.worker_generation = lease.worker_generation;
        self.lease_owner = lease.lease_owner;
        self.parked_deliveries.clear();
        Ok(())
    }

    pub fn execute<P: CommittedRuntimeApplicationPort<Protocol = D::Protocol>>(
        &mut self,
        port: &mut P,
        publish: &mut dyn FnMut(CoreHostOutput<P::Protocol>),
    ) -> Result<(), RuntimeDriverError<D::Error>>
    where
        D: Send,
        D::Error: Send,
    {
        execute_committed_with_driver(
            &mut self.driver,
            &self.worker_generation,
            &self.lease_owner,
            self.delivery_timing,
            self.delivery_clock.as_ref(),
            self.delivery_abort.as_ref(),
            &mut self.parked_deliveries,
            port,
            publish,
        )
    }
}

fn validate_delivery_timing(timing: RuntimeEffectDeliveryTiming) {
    assert!(!timing.lease_duration.is_zero());
    assert!(!timing.heartbeat_cadence.is_zero());
    assert!(
        timing.heartbeat_cadence < timing.lease_duration / 2,
        "effect-delivery heartbeat cadence must leave more than half the lease as safety margin"
    );
}


static LAST_EPOCH_MILLIS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn system_epoch_millis(observed: std::time::SystemTime) -> i64 {
    observed
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn observe_epoch_millis(clock: &std::sync::atomic::AtomicI64, observed_at_ms: i64) -> i64 {
    let previous = clock.fetch_max(observed_at_ms, std::sync::atomic::Ordering::Relaxed);
    observed_at_ms.max(previous)
}

/// Returns an epoch timestamp that never decreases within this Runtime
/// process, while observing actual wall epoch for cross-process persistence.
/// Absolute durable leases may expire after a wall-clock forward step. Instant
/// is only a local waiting clock, never a shared persistence epoch. A new
/// process behind the committed frontier must be rejected by the Store fence;
/// timestamps never replace the exact generation/token ownership witness.
pub fn monotonic_epoch_millis() -> i64 {
    let observed_at_ms = system_epoch_millis(std::time::SystemTime::now());
    observe_epoch_millis(&LAST_EPOCH_MILLIS, observed_at_ms)
}

fn claim<P: CommittedRuntimeApplicationPort>(
    port: &mut P,
    worker_generation: &str,
    lease_owner: &str,
    timing: RuntimeEffectDeliveryTiming,
    clock: &(dyn Fn() -> i64 + Send + Sync),
) -> Result<Vec<Request<P::Protocol>>, String> {
    let claimed_at_ms = clock();
    port.claim_committed_requests(RuntimeEffectDeliveryClaim {
        worker_generation: worker_generation.to_string(),
        lease_owner: lease_owner.to_string(),
        claimed_at_ms,
        lease_expires_at_ms: claimed_at_ms.saturating_add(duration_millis(timing.lease_duration)),
        limit: 64,
    })
}

fn duration_millis(duration: std::time::Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

fn execute_committed_with_driver<D, P>(
    driver: &mut D,
    worker_generation: &str,
    lease_owner: &str,
    timing: RuntimeEffectDeliveryTiming,
    clock: &(dyn Fn() -> i64 + Send + Sync),
    delivery_abort: Option<&std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
    parked_deliveries: &mut std::collections::HashMap<String, ParkedDelivery>,
    port: &mut P,
    publish: &mut dyn FnMut(CoreHostOutput<P::Protocol>),
) -> Result<(), RuntimeDriverError<D::Error>>
where
    D: HostDriver<Protocol = P::Protocol> + Send,
    D::Error: Send,
    P: CommittedRuntimeApplicationPort,
{
    let boundary_source = driver.boundary_submission_source();
    // A synchronous permission wait can return before the periodic boundary
    // poll, without emitting another response. Admit that same boundary input
    // before claiming controls, including when the prior batch became empty.
    let mut boundary_error = None;
    apply_boundary_submissions(boundary_source.as_ref(), port, publish, &mut boundary_error);
    if let Some(error) = boundary_error {
        return Err(RuntimeDriverError::Core(error));
    }
    let mut requests =
        claim(port, worker_generation, lease_owner, timing, clock).map_err(RuntimeDriverError::Core)?;
    loop {
        if requests.is_empty() {
            return Ok(());
        }
        let executing = requests
            .iter()
            .map(|request| request.effect_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut accepted_response_count = 0_usize;
        let claimed_deliveries = requests
            .iter()
            .filter_map(|request| {
                request.delivery.clone().map(|delivery| {
                    (
                        request.effect_id.0.clone(),
                        (request.request_id.clone(), delivery),
                    )
                })
            })
            .collect::<std::collections::HashMap<_, _>>();
        let dispatch_result = dispatch_with_lease_renewal(
            driver,
            requests,
            claimed_deliveries,
            parked_deliveries,
            worker_generation,
            lease_owner,
            timing,
            clock,
            delivery_abort,
            port,
            boundary_source.as_ref(),
            publish,
            &mut accepted_response_count,
        );
        let dispatch_error = match dispatch_result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(RuntimeDriverError::Host(error)),
            Err(error) => Some(error),
        };
        if let Some(error) = dispatch_error {
            // A committed interrupt can revoke an application's leased start
            // barrier while that exact dispatch is entering it. Its durable
            // cancellation still needs execution. Only drain controls targeting
            // this failed batch; never continue ordinary work after an error.
            let controls = port.load_committed_requests().map_err(RuntimeDriverError::Core)?;
            let only_exact_cancellation = |items: &[Request<P::Protocol>]| {
                !items.is_empty() && items.iter().all(|request|
                    request.payload.cancellation_target_effect_id().is_some_and(|target|
                        executing.iter().any(|effect| effect.0 == target)))
            };
            if !only_exact_cancellation(&controls) { return Err(error); }
            let controls = claim(port, worker_generation, lease_owner, timing, clock)
                .map_err(RuntimeDriverError::Core)?;
            if !only_exact_cancellation(&controls) { return Err(error); }
            requests = controls;
            continue;
        }
        let mut successors = claim(port, worker_generation, lease_owner, timing, clock)
            .map_err(RuntimeDriverError::Core)?;
        let unsettled_effect_ids = successors
            .iter()
            .filter(|request| executing.contains(&request.effect_id))
            .map(|request| request.effect_id.0.clone())
            .collect::<Vec<_>>();
        let mut scheduled = std::collections::HashSet::new();
        successors.retain(|request| {
            !executing.contains(&request.effect_id) && scheduled.insert(request.effect_id.clone())
        });
        requests = successors;
        if requests.is_empty() {
            if !unsettled_effect_ids.is_empty() {
                if accepted_response_count == 0 {
                    return Err(RuntimeDriverError::ProtocolStalled {
                        effect_ids: unsettled_effect_ids,
                    });
                }
                // A streaming Host may deliberately yield after committing a
                // bounded delta batch. The durable request remains claimable;
                // a supervisor wake-up resumes it without consuming a turn.
                return Ok(());
            }
            return Ok(());
        }
    }
}

type ClaimedDelivery = (String, EffectDeliveryLease);
#[derive(Clone)]
enum ParkedReason {
    Permission(String),
    CodeCell,
}
type ParkedDelivery = (ClaimedDelivery, ParkedReason);

// Control-plane scheduling is independent of delivery-lease liveness. A Host
// can remain silent while waiting for headers, a process, or permission. This
// is a polling interval, not an end-to-end latency guarantee (Application
// admission and source I/O still run synchronously).
const BOUNDARY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

enum ResponseAdmissionError {
    Effect(String),
    DispatchScope(String),
}

impl ResponseAdmissionError {
    fn message(&self) -> &str {
        match self {
            Self::Effect(message) | Self::DispatchScope(message) => message,
        }
    }
}

fn close_dispatch_scope(
    error: &str,
    scope_error: &mut Option<String>,
    effect_ids: &std::collections::HashSet<String>,
    active: &mut std::collections::HashMap<String, std::time::Instant>,
    closed: &mut std::collections::HashMap<String, String>,
    delivery_abort: Option<&std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
) {
    scope_error.get_or_insert_with(|| error.to_string());
    for effect_id in effect_ids {
        if let std::collections::hash_map::Entry::Vacant(entry) = closed.entry(effect_id.clone()) {
            entry.insert(error.to_string());
            if let Some(abort) = delivery_abort {
                abort(effect_id);
            }
        }
    }
    active.clear();
}

fn renew_due_deliveries<P: CommittedRuntimeApplicationPort>(
    active: &mut std::collections::HashMap<String, std::time::Instant>,
    deliveries: &mut std::collections::HashMap<String, ClaimedDelivery>,
    parked_permission_ids: &std::collections::HashMap<String, ParkedReason>,
    permission_wait_effects: &std::collections::HashMap<String, String>,
    cadence: std::time::Duration,
    lease_duration_ms: i64,
    clock: &(dyn Fn() -> i64 + Send + Sync),
    delivery_abort: Option<&std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
    port: &mut P,
    first_admission_error: &mut Option<String>,
    closed: &mut std::collections::HashMap<String, String>,
) {
    let now = std::time::Instant::now();
    let due = active
        .iter()
        .filter_map(|(id, deadline)| (*deadline <= now).then(|| id.clone()))
        .collect::<Vec<_>>();
    for effect_id in due {
        let Some((_, current)) = deliveries.get(&effect_id).cloned() else {
            let error = "active effect delivery lost its claimed lease".to_string();
            first_admission_error.get_or_insert_with(|| error.clone());
            closed.insert(effect_id.clone(), error);
            active.remove(&effect_id);
            if let Some(abort) = delivery_abort {
                abort(&effect_id);
            }
            continue;
        };
        let renewed_at_ms = clock();
        let requested_expiry = renewed_at_ms.saturating_add(lease_duration_ms);
        match port.renew_effect_delivery(RuntimeEffectDeliveryRenewal {
            delivery: current.clone(),
            renewed_at_ms,
            lease_expires_at_ms: requested_expiry,
        }) {
            Ok(renewed)
                if same_delivery_identity(&current, &renewed)
                    && renewed.lease_expires_at_ms > current.lease_expires_at_ms
                    && renewed.lease_expires_at_ms > renewed_at_ms =>
            {
                if let Some((_, lease)) = deliveries.get_mut(&effect_id) {
                    *lease = renewed;
                }
                // Permission suspension renews its parked parent atomically.
                // Mirror only the acknowledged expiry, never a speculative one.
                let linked_permission_id = permission_wait_effects.get(&effect_id);
                for (parked_effect_id, reason) in parked_permission_ids {
                    let ParkedReason::Permission(permission_id) = reason else { continue; };
                    if parked_effect_id == &effect_id || linked_permission_id != Some(permission_id)
                    {
                        continue;
                    }
                    if let Some((_, lease)) = deliveries.get_mut(parked_effect_id) {
                        if lease.worker_generation == current.worker_generation
                            && lease.lease_owner == current.lease_owner
                            && lease.lease_expires_at_ms < requested_expiry
                        {
                            lease.lease_expires_at_ms = requested_expiry;
                        }
                    }
                }
                active.insert(effect_id, std::time::Instant::now() + cadence);
            }
            Ok(_) => {
                let error =
                    "Application returned an invalid effect-delivery renewal ACK".to_string();
                first_admission_error.get_or_insert_with(|| error.clone());
                closed.insert(effect_id.clone(), error);
                active.remove(&effect_id);
                if let Some(abort) = delivery_abort {
                    abort(&effect_id);
                }
            }
            Err(error) => {
                first_admission_error.get_or_insert_with(|| error.clone());
                closed.insert(effect_id.clone(), error);
                active.remove(&effect_id);
                if let Some(abort) = delivery_abort {
                    abort(&effect_id);
                }
            }
        }
    }
}

fn same_delivery_identity(left: &EffectDeliveryLease, right: &EffectDeliveryLease) -> bool {
    left.attempt_id == right.attempt_id
        && left.worker_generation == right.worker_generation
        && left.lease_owner == right.lease_owner
        && left.lease_token == right.lease_token
}

fn bind_latest_delivery<R>(
    response: &mut HostResponseEnvelope<R>,
    deliveries: &std::collections::HashMap<String, ClaimedDelivery>,
) -> Result<(), String> {
    match (
        deliveries.get(&response.effect_id.0),
        response.delivery.as_mut(),
    ) {
        (None, None) => Ok(()),
        (None, Some(_)) => {
            Err("Host response introduced an unclaimed effect-delivery lease".into())
        }
        (Some(_), None) => Err("Host response omitted its claimed effect-delivery lease".into()),
        (Some((request_id, expected)), Some(actual)) => {
            if response.request_id != *request_id || !same_delivery_identity(actual, expected) {
                return Err("Host response changed its effect-delivery fencing identity".into());
            }
            actual.lease_expires_at_ms = expected.lease_expires_at_ms;
            Ok(())
        }
    }
}

fn publish_acceptance<P: HostExecutionProtocol>(
    accepted: CommittedRuntimeAcceptance<P>,
    publish: &mut dyn FnMut(CoreHostOutput<P>),
) {
    publish(CoreHostOutput::Committed);
    for event in accepted.events {
        publish(CoreHostOutput::Event(event));
    }
    for trace in accepted.traces {
        publish(CoreHostOutput::Trace(trace));
    }
    if let Some(control) = accepted.control {
        publish(CoreHostOutput::Control(control));
    }
    // Successor effects are claimed from the committed outbox, never carried
    // as executable authority in a presentation acceptance.
}

fn dispatch_with_lease_renewal<D, P>(
    driver: &mut D,
    requests: Vec<Request<P::Protocol>>,
    mut deliveries: std::collections::HashMap<String, ClaimedDelivery>,
    parked_deliveries: &mut std::collections::HashMap<String, ParkedDelivery>,
    worker_generation: &str,
    lease_owner: &str,
    timing: RuntimeEffectDeliveryTiming,
    clock: &(dyn Fn() -> i64 + Send + Sync),
    delivery_abort: Option<&std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
    port: &mut P,
    boundary_source: Option<
        &std::sync::Arc<dyn BoundaryInputSource<<P::Protocol as HostExecutionProtocol>::BoundaryInput>>,
    >,
    publish: &mut dyn FnMut(CoreHostOutput<P::Protocol>),
    accepted_response_count: &mut usize,
) -> Result<Result<(), D::Error>, RuntimeDriverError<D::Error>>
where
    D: HostDriver<Protocol = P::Protocol> + Send,
    D::Error: Send,
    P: CommittedRuntimeApplicationPort,
{
    struct PendingResponse<R> {
        response: HostResponseEnvelope<R>,
        acknowledgement: std::sync::mpsc::SyncSender<Result<(), String>>,
    }

    let mut host_requests = Vec::with_capacity(requests.len());
    for request in requests {
        let handled = port.dispatch_application_request(&request, &mut |accepted| {
            *accepted_response_count = accepted_response_count.saturating_add(1);
            publish_acceptance(accepted, publish);
        }).map_err(RuntimeDriverError::Core)?;
        if !handled { host_requests.push(request); }
    }
    let requests = host_requests;

    for (effect_id, (delivery, _)) in parked_deliveries.iter() {
        deliveries
            .entry(effect_id.clone())
            .or_insert_with(|| delivery.clone());
    }
    let mut parked_permission_ids = parked_deliveries
        .keys()
        .filter_map(|effect_id| {
            parked_deliveries
                .get(effect_id)
                .map(|(_, permission_id)| (effect_id.clone(), permission_id.clone()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let permission_wait_effects = requests
        .iter()
        .filter_map(|request| request.payload.permission_wait_id()
            .map(|id| (request.effect_id.0.clone(), id.to_string())))
        .collect::<std::collections::HashMap<_, _>>();
    let cadence = timing.heartbeat_cadence;
    let lease_duration_ms = duration_millis(timing.lease_duration);
    let dispatch_effect_ids = requests.iter().map(|request| request.effect_id.0.clone())
        .chain(parked_deliveries.keys().cloned()).collect::<std::collections::HashSet<_>>();
    std::thread::scope(|scope| {
        let (response_tx, response_rx) = std::sync::mpsc::channel::<PendingResponse<<P::Protocol as HostExecutionProtocol>::Response>>();
        let worker = scope.spawn(move || {
            driver.dispatch_batch(requests, &mut |response| {
                let (acknowledgement, ack_rx) = std::sync::mpsc::sync_channel(1);
                response_tx
                    .send(PendingResponse {
                        response,
                        acknowledgement,
                    })
                    .map_err(|_| "Runtime response bridge disconnected".to_string())?;
                ack_rx
                    .recv()
                    .unwrap_or_else(|_| Err("Runtime response acknowledgement disconnected".into()))
            })
        });

        let mut active = parked_permission_ids.iter().filter_map(|(id, reason)| {
            if !matches!(reason, ParkedReason::CodeCell) { return None; }
            let (_, lease) = deliveries.get(id)?;
            // Preserve the renewal deadline across pumps. Renewing immediately
            // can ask Application for the same expiry within one clock tick,
            // which is correctly rejected as a non-advancing lease ACK.
            let due_at = lease.lease_expires_at_ms.saturating_sub(lease_duration_ms)
                .saturating_add(duration_millis(cadence));
            let delay = due_at.saturating_sub(clock()).max(0) as u64;
            Some((id.clone(), std::time::Instant::now() + std::time::Duration::from_millis(delay)))
        })
            .collect::<std::collections::HashMap<_, _>>();
        let mut closed = std::collections::HashMap::<String, String>::new();
        let mut first_admission_error: Option<String> = None;
        let mut scope_error: Option<String> = None;
        let mut next_control_observation = std::time::Instant::now();
        loop {
            // Only committed cancellation effects may interrupt an in-flight
            // physical worker. Waiting for this batch to return before reading
            // them deadlocks idle streams and synchronous permission waits.
            // This cooperative signal is not a cancellation confirmation.
            if scope_error.is_none() && std::time::Instant::now() >= next_control_observation {
                next_control_observation = std::time::Instant::now() + BOUNDARY_POLL_INTERVAL;
                let controls = (|| -> Result<(), String> {
                    let committed = port.load_committed_requests()?;
                    for request in &committed {
                        if let Some(target) = request.payload.cancellation_target_effect_id() {
                            if dispatch_effect_ids.contains(target) {
                                if let Some(abort) = delivery_abort { abort(target); }
                            }
                        }
                    }
                    let now_ms = clock();
                    let controls = port.claim_application_control_requests(RuntimeEffectDeliveryClaim {
                        worker_generation: worker_generation.into(), lease_owner: lease_owner.into(),
                        claimed_at_ms: now_ms,
                        lease_expires_at_ms: now_ms.saturating_add(lease_duration_ms), limit:64,
                    }, &committed)?;
                    for control in controls {
                        let handled = port.dispatch_application_request(&control, &mut |accepted| {
                            *accepted_response_count = accepted_response_count.saturating_add(1);
                            publish_acceptance(accepted, publish);
                        })?;
                        if !handled { return Err("Core Application control claim returned a physical Host effect".into()); }
                    }
                    Ok(())
                })();
                match controls {
                    Ok(()) => {}
                    Err(error) => {
                        first_admission_error.get_or_insert_with(|| error.clone());
                        close_dispatch_scope(&error, &mut scope_error, &dispatch_effect_ids,
                            &mut active, &mut closed, delivery_abort);
                    }
                }
            }
            // A busy stream can deliver a frame before every recv_timeout.
            // Heartbeats are deadline-driven, not idle-stream-driven.
            renew_due_deliveries(
                &mut active,
                &mut deliveries,
                &parked_permission_ids,
                &permission_wait_effects,
                cadence,
                lease_duration_ms,
                clock,
                delivery_abort,
                port,
                &mut first_admission_error,
                &mut closed,
            );
            let now = std::time::Instant::now();
            let lease_wait = active
                .values()
                .min()
                .map(|deadline| deadline.saturating_duration_since(now))
                .unwrap_or(cadence);
            // An effect-local rejection is reported at dispatch completion,
            // but must not disable control admission for healthy siblings.
            // Only a failed control-plane scope closes this input source.
            let wait = if scope_error.is_none() {
                lease_wait.min(BOUNDARY_POLL_INTERVAL)
            } else {
                lease_wait
            };
            match response_rx.recv_timeout(wait) {
                Ok(mut pending) => {
                    let response_effect_id = pending.response.effect_id.0.clone();
                    let result = if let Some(error) = scope_error.as_ref() {
                        Err(ResponseAdmissionError::DispatchScope(error.clone()))
                    } else if let Some(error) = closed.get(&response_effect_id) {
                        Err(ResponseAdmissionError::Effect(error.clone()))
                    } else {
                        (|| {
                            bind_latest_delivery(&mut pending.response, &deliveries)
                                .map_err(ResponseAdmissionError::Effect)?;
                            let is_model_delta = pending.response.payload.is_model_delta();
                            if !is_model_delta {
                                let mut boundary_error = None;
                                apply_boundary_submissions(
                                    boundary_source,
                                    port,
                                    publish,
                                    &mut boundary_error,
                                );
                                if let Some(error) = boundary_error {
                                    return Err(ResponseAdmissionError::DispatchScope(error));
                                }
                            }
                            let effect_id = pending.response.effect_id.0.clone();
                            let execution_started = pending.response.payload.execution_started();
                            let terminal = pending.response.payload.is_terminal();
                            let permission_suspension = pending.response.payload
                                .permission_suspension_id()
                                .map(|id| (effect_id.clone(), id.to_string()));
                            let code_suspension = pending.response.payload.code_cell_suspension_id().is_some();
                            let accepted = port.accept_raw_host_response(pending.response)
                                .map_err(ResponseAdmissionError::Effect)?;
                            *accepted_response_count = accepted_response_count.saturating_add(1);
                            publish_acceptance(accepted, publish);
                            if execution_started && deliveries.contains_key(&effect_id) {
                                active
                                    .insert(effect_id.clone(), std::time::Instant::now() + cadence);
                            }
                            if terminal {
                                active.remove(&effect_id);
                                parked_permission_ids.remove(&effect_id);
                            } else if let Some((effect_id, permission_id)) = permission_suspension {
                                parked_permission_ids.insert(effect_id, ParkedReason::Permission(permission_id));
                            } else if code_suspension {
                                parked_permission_ids.insert(effect_id.clone(), ParkedReason::CodeCell);
                            } else if matches!(parked_permission_ids.get(&effect_id), Some(ParkedReason::Permission(_))) {
                                // The backend has resumed. Its current dispatch
                                // still owns the local fence; only another
                                // PermissionRequested parks it again.
                                parked_permission_ids.remove(&effect_id);
                            }
                            if is_model_delta {
                                let mut boundary_error = None;
                                apply_boundary_submissions(
                                    boundary_source,
                                    port,
                                    publish,
                                    &mut boundary_error,
                                );
                                if let Some(error) = boundary_error {
                                    return Err(ResponseAdmissionError::DispatchScope(error));
                                }
                            }
                            Ok(())
                        })()
                    };
                    if let Err(error) = &result {
                        first_admission_error.get_or_insert_with(|| error.message().to_string());
                        match error {
                            ResponseAdmissionError::DispatchScope(message) => close_dispatch_scope(
                                message, &mut scope_error, &dispatch_effect_ids,
                                &mut active, &mut closed, delivery_abort,
                            ),
                            ResponseAdmissionError::Effect(message) => {
                                let newly_closed = !closed.contains_key(&response_effect_id);
                                closed.entry(response_effect_id.clone()).or_insert_with(|| message.clone());
                                active.remove(&response_effect_id);
                                if newly_closed {
                                    if let Some(abort) = delivery_abort {
                                        abort(&response_effect_id);
                                    }
                                }
                            }
                        }
                    }
                    let _ = pending.acknowledgement.send(result.map_err(|error| error.message().to_string()));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if scope_error.is_none() {
                        let mut boundary_error = None;
                        apply_boundary_submissions(
                            boundary_source,
                            port,
                            publish,
                            &mut boundary_error,
                        );
                        if let Some(error) = boundary_error {
                            first_admission_error.get_or_insert_with(|| error.clone());
                            // A failed control admission has no single response
                            // owner. Stop this dispatch scope, preserving the
                            // unacknowledged input batch for exact replay. Do
                            // not convert the abort request into execution or
                            // cancellation evidence; Application owns that.
                            close_dispatch_scope(
                                &error, &mut scope_error, &dispatch_effect_ids,
                                &mut active, &mut closed, delivery_abort,
                            );
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let dispatch_result = match worker.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        parked_deliveries.clear();
        parked_deliveries.extend(parked_permission_ids.into_iter().filter_map(
            |(effect_id, permission_id)| {
                deliveries
                    .get(&effect_id)
                    .cloned()
                    .map(|delivery| (effect_id, (delivery, permission_id)))
            },
        ));
        if let Some(error) = first_admission_error {
            Err(RuntimeDriverError::Core(error))
        } else {
            Ok(dispatch_result)
        }
    })
}

fn apply_boundary_submissions<P: CommittedRuntimeApplicationPort>(
    boundary_source: Option<
        &std::sync::Arc<dyn BoundaryInputSource<<P::Protocol as HostExecutionProtocol>::BoundaryInput>>,
    >,
    port: &mut P,
    publish: &mut dyn FnMut(CoreHostOutput<P::Protocol>),
    acceptance_error: &mut Option<String>,
) {
    let Some(source) = boundary_source else {
        return;
    };
    let pending = match source.take_pending() {
        Ok(pending) => pending,
        Err(error) => { *acceptance_error = Some(error); return; }
    };
    let mut pending = pending.into_iter();
    while let Some(submission) = pending.next() {
        let result = match port.accept_boundary_input(submission.clone()) {
            Ok(accepted) => {
                publish_acceptance(accepted, publish);
                source.did_apply(&submission)
            }
            Err(error) => Err(error),
        };
        if let Err(mut error) = result {
            // take_pending may remove a complete batch from an in-memory
            // source. Return every not-ACKed item, not just the failing one.
            // A lost source ACK can replay an already committed item: the
            // Application boundary owns its stable identity/deduplication.
            for unacknowledged in std::iter::once(submission).chain(pending) {
                if let Err(release) = source.requeue(&unacknowledged) {
                    error.push_str(&format!("; release boundary lease: {release}"));
                }
            }
            *acceptance_error = Some(error);
            return;
        }
    }
}

#[cfg(test)]
mod clock_tests {
    use super::{observe_epoch_millis, system_epoch_millis};
    use std::sync::{
        atomic::{AtomicI64, AtomicUsize, Ordering},
        Arc, Barrier,
    };

    #[test]
    fn durable_epoch_observes_wall_steps_instead_of_a_process_start_anchor() {
        let clock = AtomicI64::new(0);
        let observed = |millis| {
            system_epoch_millis(std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis))
        };
        assert_eq!(observe_epoch_millis(&clock, observed(1_000_000)), 1_000_000);
        // A later Host's wall epoch is immediately visible to this long-lived
        // process too; no independent Instant anchor leaves it 60s behind.
        assert_eq!(observe_epoch_millis(&clock, observed(1_060_000)), 1_060_000);
        assert_eq!(observe_epoch_millis(&clock, observed(1_010_000)), 1_060_000);
    }

    #[test]
    fn system_epoch_projection_handles_before_epoch_and_millisecond_precision() {
        assert_eq!(
            system_epoch_millis(std::time::UNIX_EPOCH - std::time::Duration::from_secs(1)),
            0
        );
        assert_eq!(
            system_epoch_millis(std::time::UNIX_EPOCH + std::time::Duration::from_millis(123)),
            123
        );
    }

    #[test]
    fn epoch_observations_never_move_backwards() {
        let clock = AtomicI64::new(0);

        assert_eq!(observe_epoch_millis(&clock, 100), 100);
        assert_eq!(observe_epoch_millis(&clock, 99), 100);
        assert_eq!(observe_epoch_millis(&clock, 101), 101);
    }

    #[test]
    fn concurrent_epoch_observations_share_one_floor() {
        let clock = Arc::new(AtomicI64::new(0));
        let turn = Arc::new(AtomicUsize::new(0));
        let observations = [(100, 100), (200, 200), (150, 200), (250, 250), (1, 250)];
        let barrier = Arc::new(Barrier::new(observations.len() + 1));
        let workers = observations
            .into_iter()
            .enumerate()
            .map(|(index, (observed, expected))| {
                let clock = Arc::clone(&clock);
                let turn = Arc::clone(&turn);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    while turn.load(Ordering::SeqCst) != index {
                        std::thread::yield_now();
                    }
                    assert_eq!(observe_epoch_millis(&clock, observed), expected);
                    turn.fetch_add(1, Ordering::SeqCst);
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();

        for worker in workers {
            worker.join().expect("clock worker panicked");
        }
        assert_eq!(clock.load(Ordering::Relaxed), 250);
    }
}
