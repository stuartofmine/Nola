// SPDX-License-Identifier: Apache-2.0
//! A complete execution pump consumer with no Core/Application/Store types.

use nola_host_protocol::{
    BoundaryInputSource, EffectDeliveryLease, HostDriver, HostEffectId, HostExecutionProtocol,
    HostId, HostRequestEnvelope, HostResponseEnvelope, RuntimeRequestPayload,
    RuntimeResponsePayload,
};
use nola_runtime_driver::{
    CommittedRuntimeAcceptance, CommittedRuntimeApplicationPort, CommittedRuntimeDriver,
    CoreHostOutput, RuntimeDriverError, RuntimeEffectDeliveryClaim, RuntimeEffectDeliveryRenewal,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct OpaqueProtocol;
#[derive(Clone, Debug)]
enum Request { Work, Cancel(&'static str) }
#[derive(Clone, Copy, Debug)]
enum Response {
    Started,
    Delta,
    Finished,
}
impl RuntimeRequestPayload for Request {
    fn cancellation_target_effect_id(&self) -> Option<&str> {
        match self { Self::Cancel(target) => Some(target), Self::Work => None }
    }
    fn permission_wait_id(&self) -> Option<&str> {
        None
    }
}
impl RuntimeResponsePayload for Response {
    fn code_cell_suspension_id(&self) -> Option<&str> { None }
    fn is_model_delta(&self) -> bool {
        matches!(self, Self::Delta)
    }
    fn execution_started(&self) -> bool {
        matches!(self, Self::Started)
    }
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished)
    }
    fn permission_suspension_id(&self) -> Option<&str> {
        None
    }
}
impl HostExecutionProtocol for OpaqueProtocol {
    type Request = Request;
    type Response = Response;
    type BoundaryInput = u32;
    type Event = ();
    type Trace = ();
    type Control = &'static str;
}

#[derive(Default)]
struct Boundary {
    pending: Mutex<Vec<u32>>,
    acknowledged: Mutex<Vec<u32>>,
    applied_signal: Mutex<Option<std::sync::mpsc::Sender<u32>>>,
    take_count: Mutex<usize>,
    fail_take: Mutex<bool>,
}
impl BoundaryInputSource<u32> for Boundary {
    fn take_pending(&self) -> Result<Vec<u32>, String> {
        *self.take_count.lock().unwrap() += 1;
        if *self.fail_take.lock().unwrap() {
            return Err("source unavailable".into());
        }
        Ok(std::mem::take(&mut *self.pending.lock().unwrap()))
    }
    fn did_apply(&self, value: &u32) -> Result<(), String> {
        self.acknowledged.lock().unwrap().push(*value);
        if let Some(sender) = self.applied_signal.lock().unwrap().as_ref() {
            let _ = sender.send(*value);
        }
        Ok(())
    }
    fn requeue(&self, value: &u32) -> Result<(), String> {
        self.pending.lock().unwrap().push(*value);
        Ok(())
    }
}

struct Executor {
    boundary: Arc<Boundary>,
    effects: Arc<Mutex<Vec<&'static str>>>,
}
impl HostDriver for Executor {
    type Protocol = OpaqueProtocol;
    type Error = String;
    fn boundary_submission_source(&self) -> Option<Arc<dyn BoundaryInputSource<u32>>> {
        Some(self.boundary.clone())
    }
    fn dispatch(
        &mut self,
        request: HostRequestEnvelope<Request>,
        emit: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> {
        let response = |sequence, payload| HostResponseEnvelope {
            attempt_id: request.attempt_id.clone(),
            delegated_invocation_id: None,
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: None,
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery.clone(),
            sequence,
            payload,
        };
        emit(response(1, Response::Started))?;
        self.effects
            .lock()
            .unwrap()
            .push("physical-side-effect-after-ACK");
        emit(response(2, Response::Finished))
    }
}

struct Application {
    claimed: bool,
    reject_input: Option<u32>,
    accepted_inputs: Vec<u32>,
    effects: Arc<Mutex<Vec<&'static str>>>,
}
impl CommittedRuntimeApplicationPort for Application {
    type Protocol = OpaqueProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        Ok(vec![])
    }
    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        if self.claimed {
            return Ok(vec![]);
        }
        self.claimed = true;
        Ok(vec![HostRequestEnvelope {
            attempt_started: false,
            attempt_id: Some("attempt".into()),
            delegated_invocation_id: None,
            request_id: "request".into(),
            effect_id: HostEffectId("effect".into()),
            host_id: HostId("host".into()),
            lease_id: None,
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            catalog_revision: 1,
            availability_epoch: 1,
            created_at_ms: 1,
            deadline_ms: None,
            batch: None,
            resume_after_sequence: None,
            delivery: Some(EffectDeliveryLease {
                attempt_id: "attempt".into(),
                worker_generation: claim.worker_generation,
                lease_owner: claim.lease_owner,
                lease_token: "lease".into(),
                lease_expires_at_ms: claim.lease_expires_at_ms,
            }),
            payload: Request::Work,
        }])
    }
    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String> {
        Ok(EffectDeliveryLease {
            lease_expires_at_ms: renewal.lease_expires_at_ms,
            ..renewal.delivery
        })
    }
    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope<Response>,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.effects.lock().unwrap().push(match response.payload {
            Response::Started => "committed-start",
            Response::Delta => "committed-delta",
            Response::Finished => "committed-result",
        });
        Ok(CommittedRuntimeAcceptance {
            events: vec![],
            traces: vec![],
            control: Some("committed"),
        })
    }
    fn accept_boundary_input(
        &mut self,
        input: u32,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        if self.reject_input == Some(input) {
            return Err("input rejected".into());
        }
        self.accepted_inputs.push(input);
        Ok(CommittedRuntimeAcceptance {
            events: vec![],
            traces: vec![],
            control: None,
        })
    }
}

#[test]
fn opaque_protocol_keeps_commit_ack_before_side_effect_and_boundary_ack() {
    let boundary = Arc::new(Boundary::default());
    *boundary.pending.lock().unwrap() = vec![1, 2];
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: false,
        reject_input: None,
        accepted_inputs: vec![],
        effects: effects.clone(),
    };
    let mut pump = test_driver(Executor {
        boundary: boundary.clone(),
        effects: effects.clone(),
    });
    let mut publications = 0;
    pump.execute(&mut application, &mut |output| {
        if matches!(output, CoreHostOutput::Committed) {
            publications += 1;
        }
    })
    .unwrap();
    assert_eq!(application.accepted_inputs, vec![1, 2]);
    assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![1, 2]);
    assert_eq!(
        *effects.lock().unwrap(),
        vec![
            "committed-start",
            "physical-side-effect-after-ACK",
            "committed-result"
        ]
    );
    assert_eq!(publications, 4);
}

/// Application reads and physical effects share the committed pump, but not
/// the executor. A mixed batch must neither lose its Host sibling nor reopen
/// storage through the Host.
struct ObservationApplication(Application);
impl CommittedRuntimeApplicationPort for ObservationApplication {
    type Protocol = OpaqueProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        self.0.load_committed_requests()
    }
    fn claim_committed_requests(&mut self, claim: RuntimeEffectDeliveryClaim)
        -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        let mut requests = self.0.claim_committed_requests(claim)?;
        if let Some(request) = requests.first() {
            let mut observation = request.clone();
            observation.effect_id = HostEffectId("observation".into());
            observation.request_id = "observation-request".into();
            requests.insert(0, observation);
        }
        Ok(requests)
    }
    fn dispatch_application_request(&mut self, request: &HostRequestEnvelope<Request>,
        publish: &mut dyn FnMut(CommittedRuntimeAcceptance<OpaqueProtocol>)) -> Result<bool, String> {
        if request.effect_id.0 != "observation" { return Ok(false); }
        self.0.effects.lock().unwrap().push("application-owned-observation");
        publish(CommittedRuntimeAcceptance { events: vec![], traces: vec![], control: None });
        Ok(true)
    }
    fn renew_effect_delivery(&mut self, renewal: RuntimeEffectDeliveryRenewal) -> Result<EffectDeliveryLease, String> {
        self.0.renew_effect_delivery(renewal)
    }
    fn accept_raw_host_response(&mut self, response: HostResponseEnvelope<Response>)
        -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        assert_ne!(response.effect_id.0, "observation", "Application read reached physical Host");
        self.0.accept_raw_host_response(response)
    }
    fn accept_boundary_input(&mut self, input: u32) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.0.accept_boundary_input(input)
    }
}

#[test]
fn application_observation_uses_current_owner_and_preserves_host_sibling() {
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = ObservationApplication(Application {
        claimed: false, reject_input: None, accepted_inputs: vec![], effects: effects.clone(),
    });
    let mut pump = test_driver(Executor {
        boundary: Arc::new(Boundary::default()), effects: effects.clone(),
    });
    let mut committed = 0;
    pump.execute(&mut application, &mut |output| {
        if matches!(output, CoreHostOutput::Committed) { committed += 1; }
    }).unwrap();
    assert_eq!(*effects.lock().unwrap(), vec![
        "application-owned-observation", "committed-start",
        "physical-side-effect-after-ACK", "committed-result",
    ]);
    assert_eq!(committed, 3);
}

#[test]
fn opaque_boundary_failure_preserves_unacknowledged_batch_in_order() {
    let boundary = Arc::new(Boundary::default());
    *boundary.pending.lock().unwrap() = vec![1, 2, 3];
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: false,
        reject_input: Some(2),
        accepted_inputs: vec![],
        effects: effects.clone(),
    };
    let mut pump = test_driver(Executor {
        boundary: boundary.clone(),
        effects: effects.clone(),
    });
    assert!(matches!(
        pump.execute(&mut application, &mut |_| {}),
        Err(RuntimeDriverError::Core(_))
    ));
    assert_eq!(application.accepted_inputs, vec![1]);
    assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![1]);
    assert_eq!(*boundary.pending.lock().unwrap(), vec![2, 3]);
    assert!(
        effects.lock().unwrap().is_empty(),
        "rejected input cannot ACK execution start"
    );
}

#[test]
fn initial_boundary_is_admitted_even_when_the_first_claim_is_empty() {
    // Models the second pump after a synchronous permission wait returned
    // without a response before its periodic poll. This tests pump ownership,
    // not cancellation completion; real seals are checked by the native E2E.
    let boundary = Arc::new(Boundary::default());
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: true, reject_input: None, accepted_inputs: vec![], effects: effects.clone(),
    };
    let mut pump = test_driver(Executor {
        boundary: boundary.clone(), effects: effects.clone(),
    });
    pump.execute(&mut application, &mut |_| {}).unwrap();
    boundary.pending.lock().unwrap().push(9);
    pump.execute(&mut application, &mut |_| {}).unwrap();
    pump.execute(&mut application, &mut |_| {}).unwrap();
    assert_eq!(application.accepted_inputs, vec![9]);
    assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![9]);
    assert!(boundary.pending.lock().unwrap().is_empty());
    assert!(effects.lock().unwrap().is_empty(), "no business effect may be replayed to deliver a boundary ACK");
}

#[test]
fn initial_boundary_rejection_is_preserved_on_retry_without_claim_or_ack() {
    let boundary = Arc::new(Boundary::default());
    boundary.pending.lock().unwrap().push(9);
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: false, reject_input: Some(9), accepted_inputs: vec![], effects: effects.clone(),
    };
    let mut pump = test_driver(Executor {
        boundary: boundary.clone(), effects: effects.clone(),
    }).with_delivery_abort(Arc::new(|_| panic!("no effect was claimed before initial rejection")));
    for _ in 0..2 {
        assert!(matches!(pump.execute(&mut application, &mut |_| {}),
            Err(RuntimeDriverError::Core(ref error)) if error == "input rejected"));
        assert!(!application.claimed, "a rejected initial control cannot dispatch work");
        assert!(application.accepted_inputs.is_empty());
        assert!(boundary.acknowledged.lock().unwrap().is_empty());
        assert_eq!(*boundary.pending.lock().unwrap(), vec![9]);
        assert!(effects.lock().unwrap().is_empty());
    }
}

/// This Host emits no frames until its boundary input has been admitted (or
/// its delivery has been cooperatively aborted). The timeout is a test escape
/// hatch, not a production deadline or measured performance assertion.
struct SilentExecutor {
    boundary: Arc<Boundary>,
    wake: std::sync::mpsc::Receiver<u32>,
    input: u32,
}

impl HostDriver for SilentExecutor {
    type Protocol = OpaqueProtocol;
    type Error = String;

    fn boundary_submission_source(&self) -> Option<Arc<dyn BoundaryInputSource<u32>>> {
        Some(self.boundary.clone())
    }

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope<Request>,
        emit: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> {
        // Input arrives after dispatch starts, before even ExecutionStarted.
        // Thus neither model deltas nor an active lease deadline can wake it.
        self.boundary.pending.lock().unwrap().push(self.input);
        self.wake.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "silent Host never observed control admission/abort".to_string())?;
        emit(HostResponseEnvelope {
            attempt_id: request.attempt_id,
            delegated_invocation_id: None,
            request_id: request.request_id,
            effect_id: request.effect_id,
            host_id: request.host_id,
            lease_id: None,
            session_id: request.session_id,
            turn_id: request.turn_id,
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery,
            sequence: 1,
            payload: Response::Finished,
        })
    }
}

#[test]
fn silent_host_consumes_boundary_without_waiting_for_lease_heartbeat() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let boundary = Arc::new(Boundary::default());
    *boundary.applied_signal.lock().unwrap() = Some(sender);
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: false, reject_input: None, accepted_inputs: vec![], effects: effects.clone(),
    };
    let mut pump = test_driver(SilentExecutor {
        boundary: boundary.clone(), wake: receiver, input: 7,
    });
    let mut committed = 0;
    pump.execute(&mut application, &mut |output| {
        if matches!(output, CoreHostOutput::Committed) { committed += 1; }
    }).unwrap();
    assert_eq!(application.accepted_inputs, vec![7]);
    assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![7]);
    assert_eq!(committed, 2, "boundary and result each publish only after admission");
    assert_eq!(*effects.lock().unwrap(), vec!["committed-result"]);
}

#[test]
fn silent_boundary_rejection_aborts_scope_without_ack_or_late_result_acceptance() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let boundary = Arc::new(Boundary::default());
    let aborted = Arc::new(Mutex::new(vec![]));
    let observed = aborted.clone();
    let effects = Arc::new(Mutex::new(vec![]));
    let mut application = Application {
        claimed: false, reject_input: Some(7), accepted_inputs: vec![], effects: effects.clone(),
    };
    let mut pump = test_driver(SilentExecutor {
        boundary: boundary.clone(), wake: receiver, input: 7,
    }).with_delivery_abort(Arc::new(move |effect| {
        observed.lock().unwrap().push(effect.to_string());
        let _ = sender.send(7);
    }));
    let mut committed = 0;
    assert!(matches!(pump.execute(&mut application, &mut |output| {
        if matches!(output, CoreHostOutput::Committed) { committed += 1; }
    }), Err(RuntimeDriverError::Core(_))));
    assert_eq!(*aborted.lock().unwrap(), vec!["effect"]);
    assert!(application.accepted_inputs.is_empty());
    assert!(boundary.acknowledged.lock().unwrap().is_empty());
    assert_eq!(*boundary.pending.lock().unwrap(), vec![7]);
    assert!(effects.lock().unwrap().is_empty());
    assert_eq!(committed, 0);
}

struct CancellationApplication {
    inner: Application,
    cancelled: bool,
    claims_after_cancel: usize,
}

impl CommittedRuntimeApplicationPort for CancellationApplication {
    type Protocol = OpaqueProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        self.inner.load_committed_requests()
    }
    fn claim_committed_requests(
        &mut self, claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        if self.cancelled {
            self.claims_after_cancel += 1;
            return Ok(vec![]);
        }
        self.inner.claim_committed_requests(claim)
    }
    fn renew_effect_delivery(
        &mut self, renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String> {
        self.inner.renew_effect_delivery(renewal)
    }
    fn accept_raw_host_response(
        &mut self, response: HostResponseEnvelope<Response>,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.inner.accept_raw_host_response(response)
    }
    fn accept_boundary_input(
        &mut self, input: u32,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        let mut accepted = self.inner.accept_boundary_input(input)?;
        // Cancellation semantics belong only to this fixture Application;
        // the pump cannot inspect or reinterpret the opaque value.
        self.cancelled = true;
        accepted.control = Some("cancel-committed");
        Ok(accepted)
    }
}

#[test]
fn silent_cancellation_publishes_commit_and_never_invents_successor_dispatch() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let boundary = Arc::new(Boundary::default());
    *boundary.applied_signal.lock().unwrap() = Some(sender);
    let mut application = CancellationApplication {
        inner: Application {
            claimed: false, reject_input: None, accepted_inputs: vec![],
            effects: Arc::new(Mutex::new(vec![])),
        },
        cancelled: false,
        claims_after_cancel: 0,
    };
    let mut pump = test_driver(SilentExecutor {
        boundary: boundary.clone(), wake: receiver, input: 9,
    });
    let mut controls = vec![];
    pump.execute(&mut application, &mut |output| {
        if let CoreHostOutput::Control(control) = output { controls.push(control); }
    }).unwrap();
    assert_eq!(controls.first(), Some(&"cancel-committed"));
    assert!(application.cancelled);
    assert_eq!(application.claims_after_cancel, 1);
    assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![9]);
    assert_eq!(application.inner.accepted_inputs, vec![9]);
}

struct TwoEffectApplication(Application);
impl CommittedRuntimeApplicationPort for TwoEffectApplication {
    type Protocol = OpaqueProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        self.0.load_committed_requests()
    }
    fn claim_committed_requests(
        &mut self, claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        let mut requests = self.0.claim_committed_requests(claim)?;
        if let Some(first) = requests.first() {
            let mut second = first.clone();
            second.effect_id = HostEffectId("effect-2".into());
            second.request_id = "request-2".into();
            second.attempt_id = Some("attempt-2".into());
            if let Some(delivery) = second.delivery.as_mut() {
                delivery.attempt_id = "attempt-2".into();
                delivery.lease_token = "lease-2".into();
            }
            requests.push(second);
        }
        Ok(requests)
    }
    fn renew_effect_delivery(
        &mut self, renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String> { self.0.renew_effect_delivery(renewal) }
    fn accept_raw_host_response(
        &mut self, response: HostResponseEnvelope<Response>,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.0.accept_raw_host_response(response)
    }
    fn accept_boundary_input(
        &mut self, input: u32,
    ) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.0.accept_boundary_input(input)
    }
}

struct LateFramesExecutor {
    boundary: Arc<Boundary>,
    first_payload: Response,
    source_failure: bool,
}
impl HostDriver for LateFramesExecutor {
    type Protocol = OpaqueProtocol;
    type Error = String;
    fn boundary_submission_source(&self) -> Option<Arc<dyn BoundaryInputSource<u32>>> {
        Some(self.boundary.clone())
    }
    fn dispatch(
        &mut self, _: HostRequestEnvelope<Request>,
        _: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> { unreachable!("fixture overrides batch") }
    fn dispatch_batch(
        &mut self, requests: Vec<HostRequestEnvelope<Request>>,
        emit: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> {
        assert_eq!(requests.len(), 2);
        assert_eq!(*self.boundary.take_count.lock().unwrap(), 1,
            "the initial empty boundary must pass before claiming these effects");
        assert!(self.boundary.pending.lock().unwrap().is_empty());
        *self.boundary.pending.lock().unwrap() = vec![7, 8];
        *self.boundary.fail_take.lock().unwrap() = self.source_failure;
        for (index, request) in requests.into_iter().enumerate() {
            // Deliberately continue emitting other in-flight effects after a
            // rejected frame, as a parallel Host can do during cancellation.
            for sequence in 1..=2 {
                assert!(emit(HostResponseEnvelope {
                    attempt_id: request.attempt_id.clone(),
                    delegated_invocation_id: None,
                    request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                    host_id: request.host_id.clone(), lease_id: None,
                    session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(), sequence,
                    payload: if index == 0 && sequence == 1 { self.first_payload } else { Response::Finished },
                }).is_err());
            }
        }
        Ok(())
    }
}

#[test]
fn boundary_failure_closes_all_effects_before_frame_or_after_delta_without_repoll() {
    for first_payload in [Response::Started, Response::Delta] {
        for source_failure in [false, true] {
            let boundary = Arc::new(Boundary::default());
            let effects = Arc::new(Mutex::new(vec![]));
            let mut application = TwoEffectApplication(Application {
                claimed: false, reject_input: Some(7), accepted_inputs: vec![], effects: effects.clone(),
            });
            let aborted = Arc::new(Mutex::new(vec![]));
            let observed = aborted.clone();
            let mut pump = test_driver(LateFramesExecutor {
                boundary: boundary.clone(), first_payload, source_failure,
            }).with_delivery_abort(Arc::new(move |effect| {
                observed.lock().unwrap().push(effect.to_string());
            }));
            assert!(matches!(pump.execute(&mut application, &mut |_| {}), Err(RuntimeDriverError::Core(_))));
            let expected = if matches!(first_payload, Response::Delta) {
                vec!["committed-delta"]
            } else { vec![] };
            assert_eq!(*effects.lock().unwrap(), expected, "no late frame from either effect may commit");
            assert!(application.0.claimed, "both effects must be claimed before the in-flight failure");
            assert_eq!(*boundary.take_count.lock().unwrap(), 2,
                "one initial empty poll and one in-flight failure; late frames must not repoll");
            assert_eq!(*boundary.pending.lock().unwrap(), vec![7, 8]);
            assert!(boundary.acknowledged.lock().unwrap().is_empty());
            let mut aborted = aborted.lock().unwrap().clone();
            aborted.sort();
            assert_eq!(aborted, vec!["effect", "effect-2"]);
        }
    }
}

struct EffectFailureThenControlExecutor {
    boundary: Arc<Boundary>,
    wake: std::sync::mpsc::Receiver<u32>,
    frame_before_wait: Option<Response>,
    reject_control: bool,
    fail_source: bool,
}

impl HostDriver for EffectFailureThenControlExecutor {
    type Protocol = OpaqueProtocol;
    type Error = String;
    fn boundary_submission_source(&self) -> Option<Arc<dyn BoundaryInputSource<u32>>> {
        Some(self.boundary.clone())
    }
    fn dispatch(
        &mut self, _: HostRequestEnvelope<Request>,
        _: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> { unreachable!("fixture overrides batch") }
    fn dispatch_batch(
        &mut self, requests: Vec<HostRequestEnvelope<Request>>,
        emit: &mut dyn FnMut(HostResponseEnvelope<Response>) -> Result<(), String>,
    ) -> Result<(), String> {
        assert_eq!(requests.len(), 2);
        let response = |index: usize, sequence, payload| {
            let request = &requests[index];
            HostResponseEnvelope {
                attempt_id: request.attempt_id.clone(), delegated_invocation_id: None,
                request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                host_id: request.host_id.clone(), lease_id: None,
                session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                delivery: request.delivery.clone(), sequence, payload,
            }
        };
        let mut invalid = response(0, 1, Response::Started);
        invalid.request_id = "wrong-request".into();
        assert!(emit(invalid).is_err());
        // This input arrives only after the local failure is acknowledged.
        // It belongs to the control plane, not the failed effect.
        *self.boundary.fail_take.lock().unwrap() = self.fail_source;
        self.boundary.pending.lock().unwrap().push(7);
        let control_fails = self.reject_control || self.fail_source;
        if let Some(payload) = self.frame_before_wait {
            assert_eq!(emit(response(1, 1, payload)).is_err(), control_fails);
        }
        // Wake on committed input or scope abort of the healthy sibling.
        // A bounded fixture escape prevents a regression hanging the suite;
        // this is not a production timeout or latency assertion.
        self.wake.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "healthy silent sibling lost its control plane".to_string())?;
        let takes_after_wake = *self.boundary.take_count.lock().unwrap();
        assert!(emit(response(0, 2, Response::Finished)).is_err());
        assert_eq!(emit(response(1, 2, Response::Finished)).is_err(), control_fails);
        if control_fails {
            assert_eq!(*self.boundary.take_count.lock().unwrap(), takes_after_wake,
                "scope failure rejects late frames without polling again");
        }
        Ok(())
    }
}

#[test]
fn effect_failure_preserves_sibling_control_admission_and_later_scope_failure() {
    for frame_before_wait in [None, Some(Response::Started), Some(Response::Delta)] {
        for (reject_control, fail_source) in [(false, false), (true, false), (false, true)] {
            let (sender, receiver) = std::sync::mpsc::channel();
            let boundary = Arc::new(Boundary::default());
            *boundary.applied_signal.lock().unwrap() = Some(sender.clone());
            let effects = Arc::new(Mutex::new(vec![]));
            let mut application = TwoEffectApplication(Application {
                claimed: false, reject_input: reject_control.then_some(7),
                accepted_inputs: vec![], effects: effects.clone(),
            });
            let aborted = Arc::new(Mutex::new(vec![]));
            let observed = aborted.clone();
            let mut pump = test_driver(EffectFailureThenControlExecutor {
                boundary: boundary.clone(), wake: receiver,
                frame_before_wait, reject_control, fail_source,
            }).with_delivery_abort(Arc::new(move |effect| {
                observed.lock().unwrap().push(effect.to_string());
                if effect == "effect-2" { let _ = sender.send(7); }
            }));
            let result = pump.execute(&mut application, &mut |_| {});
            assert!(matches!(result, Err(RuntimeDriverError::Core(ref error))
                if error == "Host response changed its effect-delivery fencing identity"),
                "the first effect failure remains the reported failure");
            let control_fails = reject_control || fail_source;
            let mut expected_effects = vec![];
            match frame_before_wait {
                Some(Response::Delta) => expected_effects.push("committed-delta"),
                Some(Response::Started) if !control_fails => expected_effects.push("committed-start"),
                _ => {}
            }
            if !control_fails { expected_effects.push("committed-result"); }
            let actual_effects = effects.lock().unwrap().clone();
            if control_fails && matches!(frame_before_wait, Some(Response::Delta)) {
                // The control timer may legitimately win before the Host
                // delta reaches the bridge. Otherwise its committed delta
                // survives the subsequent control failure.
                assert!(actual_effects.is_empty() || actual_effects == expected_effects);
            } else {
                assert_eq!(actual_effects, expected_effects);
            }
            let mut aborted = aborted.lock().unwrap().clone();
            aborted.sort();
            if control_fails {
                assert_eq!(aborted, vec!["effect", "effect-2"]);
                assert!(application.0.accepted_inputs.is_empty());
                assert!(boundary.acknowledged.lock().unwrap().is_empty());
                assert_eq!(*boundary.pending.lock().unwrap(), vec![7]);
            } else {
                assert_eq!(aborted, vec!["effect"]);
                assert_eq!(application.0.accepted_inputs, vec![7]);
                assert_eq!(*boundary.acknowledged.lock().unwrap(), vec![7]);
                assert!(boundary.pending.lock().unwrap().is_empty());
            }
        }
    }
}

fn test_driver<D: nola_host_protocol::HostDriver>(driver: D) -> nola_runtime_driver::CommittedRuntimeDriver<D> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    nola_runtime_driver::CommittedRuntimeDriver::new(driver, nola_host_protocol::HostWorkerProcessIdentity {
        pid: 1, boot_id: "fixture-boot".into(), start_id: "1".into(),
        nonce: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed).to_string(),
    }).unwrap()
}

struct CancelStartRaceApplication {
    inner: Application,
    phase: u8,
    target: Option<&'static str>,
    original: Option<HostRequestEnvelope<Request>>,
}
impl CancelStartRaceApplication {
    fn pending_control(&self) -> HostRequestEnvelope<Request> {
        let mut request = self.original.as_ref().unwrap().clone();
        request.effect_id.0 = "cancel-after-application-start-race".into();
        request.request_id = "cancel-control-request".into();
        request.payload = self.target.map_or(Request::Work, Request::Cancel);
        request
    }
}
impl CommittedRuntimeApplicationPort for CancelStartRaceApplication {
    type Protocol = OpaqueProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        Ok(if self.phase == 1 { vec![self.pending_control()] } else { vec![] })
    }
    fn claim_committed_requests(&mut self, claim: RuntimeEffectDeliveryClaim)
        -> Result<Vec<HostRequestEnvelope<Request>>, String> {
        if self.phase == 0 {
            let requests = self.inner.claim_committed_requests(claim)?;
            self.original = requests.first().cloned();
            return Ok(requests);
        }
        if self.phase != 1 { return Ok(vec![]); }
        let mut request = self.pending_control();
        request.delivery = Some(EffectDeliveryLease {
            attempt_id: "cancel-attempt".into(), worker_generation: claim.worker_generation,
            lease_owner: claim.lease_owner, lease_token: "cancel-lease".into(),
            lease_expires_at_ms: claim.lease_expires_at_ms,
        });
        Ok(vec![request])
    }
    fn dispatch_application_request(&mut self, request: &HostRequestEnvelope<Request>,
        _: &mut dyn FnMut(CommittedRuntimeAcceptance<OpaqueProtocol>)) -> Result<bool, String> {
        if request.effect_id.0 != "effect" { return Ok(false); }
        // Application committed an interrupt after this effect was claimed,
        // before the local application's start barrier could be admitted.
        self.phase = 1;
        Err("application start barrier was revoked by the committed interrupt".into())
    }
    fn renew_effect_delivery(&mut self, renewal: RuntimeEffectDeliveryRenewal) -> Result<EffectDeliveryLease, String> {
        self.inner.renew_effect_delivery(renewal)
    }
    fn accept_raw_host_response(&mut self, response: HostResponseEnvelope<Response>)
        -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        assert_eq!(response.effect_id.0, "cancel-after-application-start-race");
        if matches!(response.payload, Response::Finished) { self.phase = 2; }
        self.inner.accept_raw_host_response(response)
    }
    fn accept_boundary_input(&mut self, input: u32) -> Result<CommittedRuntimeAcceptance<OpaqueProtocol>, String> {
        self.inner.accept_boundary_input(input)
    }
}

#[test]
fn application_start_race_drains_only_committed_cancellation_of_the_failed_batch() {
    for target in [Some("effect"), Some("unrelated-effect"), None] {
        let effects = Arc::new(Mutex::new(vec![]));
        let mut application = CancelStartRaceApplication {
            inner: Application { claimed: false, reject_input: None, accepted_inputs: vec![], effects: effects.clone() },
            phase: 0, target, original: None,
        };
        let host = Executor { boundary: Arc::new(Boundary::default()), effects: effects.clone() };
        let mut runtime = CommittedRuntimeDriver::with_delivery_identity(host, "generation", "owner");
        let result = runtime.execute(&mut application, &mut |_| {});
        if target == Some("effect") {
            result.expect("the exact committed cancellation must drain despite the revoked application start barrier");
            assert_eq!(application.phase, 2);
            assert_eq!(*effects.lock().unwrap(), vec!["committed-start", "physical-side-effect-after-ACK", "committed-result"]);
        } else {
            assert!(matches!(result, Err(RuntimeDriverError::Core(_))));
            assert!(effects.lock().unwrap().is_empty(), "ordinary/foreign work must not run after dispatch failed");
        }
    }
}

#[test]
fn application_control_dispatch_does_not_wait_for_long_physical_batch() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct PendingControl {
        inner: Application,
        original: Option<HostRequestEnvelope<Request>>,
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
        claimed: bool,
    }
    impl CommittedRuntimeApplicationPort for PendingControl {
        type Protocol = OpaqueProtocol;
        fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope<Request>>, String> {
            Ok(if self.started.load(Ordering::SeqCst) && !self.claimed {
                let mut control = self.original.clone().unwrap();
                control.effect_id.0 = "application-stop-intent".into();
                control.request_id = "application-stop-request".into();
                vec![control]
            } else { vec![] })
        }
        fn claim_committed_requests(&mut self, claim:RuntimeEffectDeliveryClaim) -> Result<Vec<HostRequestEnvelope<Request>>,String> {
            let requests=self.inner.claim_committed_requests(claim)?;
            if let Some(request)=requests.first() {self.original=Some(request.clone());}
            Ok(requests)
        }
        fn claim_application_control_requests(&mut self, claim:RuntimeEffectDeliveryClaim, observed:&[HostRequestEnvelope<Request>])
            -> Result<Vec<HostRequestEnvelope<Request>>,String> {
            if observed.is_empty() {return Ok(vec![]);}
            assert!(!self.claimed); self.claimed=true;
            let mut request=observed[0].clone();
            let lease=request.delivery.as_mut().unwrap();
            lease.worker_generation=claim.worker_generation;lease.lease_owner=claim.lease_owner;
            lease.lease_expires_at_ms=claim.lease_expires_at_ms;
            Ok(vec![request])
        }
        fn dispatch_application_request(&mut self, request:&HostRequestEnvelope<Request>, publish:&mut dyn FnMut(CommittedRuntimeAcceptance<OpaqueProtocol>)) -> Result<bool,String> {
            if request.effect_id.0!="application-stop-intent" {return Ok(false);}
            assert!(self.claimed); assert!(self.started.load(Ordering::SeqCst));
            self.stopped.store(true,Ordering::SeqCst);
            publish(CommittedRuntimeAcceptance{events:vec![],traces:vec![],control:None});
            Ok(true)
        }
        fn renew_effect_delivery(&mut self,renewal:RuntimeEffectDeliveryRenewal)->Result<EffectDeliveryLease,String>{self.inner.renew_effect_delivery(renewal)}
        fn accept_raw_host_response(&mut self,response:HostResponseEnvelope<Response>)->Result<CommittedRuntimeAcceptance<OpaqueProtocol>,String>{self.inner.accept_raw_host_response(response)}
        fn accept_boundary_input(&mut self,input:u32)->Result<CommittedRuntimeAcceptance<OpaqueProtocol>,String>{self.inner.accept_boundary_input(input)}
    }
    struct LongPhysical { started:Arc<AtomicBool>, stopped:Arc<AtomicBool> }
    impl HostDriver for LongPhysical {
        type Protocol=OpaqueProtocol; type Error=String;
        fn dispatch(&mut self, request:HostRequestEnvelope<Request>, emit:&mut dyn FnMut(HostResponseEnvelope<Response>)->Result<(),String>)->Result<(),String>{
            assert_eq!(request.effect_id.0,"effect");
            self.started.store(true,Ordering::SeqCst);
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
            while !self.stopped.load(Ordering::SeqCst) {
                if std::time::Instant::now()>deadline {return Err("control starved behind physical batch".into());}
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            emit(HostResponseEnvelope{attempt_id:request.attempt_id,delegated_invocation_id:None,
                request_id:request.request_id,effect_id:request.effect_id,host_id:request.host_id,
                lease_id:None,session_id:request.session_id,turn_id:request.turn_id,
                catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,
                delivery:request.delivery,sequence:0,payload:Response::Finished})
        }
    }
    let started=Arc::new(AtomicBool::new(false)); let stopped=Arc::new(AtomicBool::new(false));
    let mut app=PendingControl{inner:Application{claimed:false,reject_input:None,accepted_inputs:vec![],effects:Arc::default()},
        original:None,started:started.clone(),stopped:stopped.clone(),claimed:false};
    let mut runtime=CommittedRuntimeDriver::with_delivery_identity(LongPhysical{started,stopped},"generation","owner");
    runtime.execute(&mut app,&mut |_|{}).unwrap();
    assert!(app.claimed);
}

#[path = "parts/mailbox_contract.rs"]
mod mailbox_contract;
