// SPDX-License-Identifier: Apache-2.0
//! Host identity/receipt consumer contracts, not OS stop integration evidence.
use super::*;
use super::effect_delivery_start_barrier_tests::{test_context, test_llm};
use nola_domain::execution_evidence::{LocalExecutionIdentity, LocalExecutionStop, LocalExecutionStopScope};
use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState, ToolPresentation, ToolResult, ToolResultContract};

struct Database(std::ffi::OsString, Option<std::ffi::OsString>);
impl Database {
    fn bind(root: &std::path::Path) -> Self {
        let key = std::ffi::OsString::from("NOLA_DB_PATH");
        let previous = std::env::var_os(&key);
        std::env::set_var(&key, root.join("stop-contract.sqlite"));
        Self(key, previous)
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        if let Some(value) = self.1.take() { std::env::set_var(&self.0, value); }
        else { std::env::remove_var(&self.0); }
    }
}

fn identity() -> LocalExecutionIdentity {
    LocalExecutionIdentity {
        execution_id: "fresh-dispatch-nonce".into(), host_id: "delivery-host".into(),
        session_id: "session-fenced".into(), turn_id: "turn-fenced".into(),
        effect_id: "effect-fenced".into(), request_id: "request-fenced".into(),
        call_id: "stop-call".into(), delivery_attempt_id: Some("attempt-fenced".into()),
    }
}

fn output(identity: LocalExecutionIdentity, state: ExecutionState) -> ToolOutcome {
    ToolOutcome {
        execution: Some(ExecutionReceipt {
            state, local_stop: Some(LocalExecutionStop {
                identity, scope: LocalExecutionStopScope::OwnedProcessGroup { leader_pid: 1234 },
            }), ..Default::default()
        }),
        data: Some(json!({"safe_to_retry":false,"state":"unknown"})),
        text: "External effects remain unknown".into(), display: "Needs reconciliation".into(),
        error: true, artifacts: Vec::new(), activation: None, full_output_ref: None,
    }
}

fn response(identity: LocalExecutionIdentity, state: ExecutionState) -> HostResponse {
    let outcome = output(identity, state);
    HostResponse::ToolCompleted {
        call_id: "stop-call".into(), output: serde_json::to_value(tool_result_contract(&outcome, None)).unwrap(),
        outcome: HostToolOutcome::Failed, activation: None, full_output_ref: None, artifacts: Vec::new(),
    }
}

#[test]
fn current_local_stop_does_not_change_unknown_or_partial_business_credit() {
    for state in [ExecutionState::Unknown, ExecutionState::Partial] {
        let expected = identity();
        let response = response(expected.clone(), state);
        assert!(terminal_response_proves_local_stop(&response, false, Some(&expected)));
        let HostResponse::ToolCompleted { output, outcome, .. } = response else { unreachable!() };
        let result: ToolResult = serde_json::from_value(output).unwrap();
        assert_eq!(result.execution.as_ref().unwrap().state, state);
        assert_eq!(outcome, HostToolOutcome::Failed);
        assert!(result.execution.as_ref().unwrap().effects.is_empty());
        assert!(result.execution.as_ref().unwrap().checks.is_empty());
        assert_eq!(result.data.as_ref().unwrap()["safe_to_retry"], false);
        assert_eq!(result.into_core_output()["_nola_execution"]["state"], serde_json::to_value(state).unwrap());
    }
}

#[test]
fn old_nonce_foreign_owner_call_effect_and_delivery_never_prove_current_stop() {
    for damage in ["nonce", "host", "session", "turn", "effect", "request", "call", "attempt", "missing_attempt", "response_call", "pid"] {
        let expected = identity();
        let mut foreign = expected.clone();
        match damage {
            "nonce" => foreign.execution_id = "previous-invocation".into(),
            "host" => foreign.host_id = "foreign".into(),
            "session" => foreign.session_id = "foreign".into(),
            "turn" => foreign.turn_id = "foreign".into(),
            "effect" => foreign.effect_id = "foreign".into(),
            "request" => foreign.request_id = "foreign".into(),
            "call" => foreign.call_id = "foreign".into(),
            "attempt" => foreign.delivery_attempt_id = Some("previous-attempt".into()),
            "missing_attempt" => foreign.delivery_attempt_id = None,
            _ => {},
        }
        let mut response = response(foreign, ExecutionState::Unknown);
        if let HostResponse::ToolCompleted { call_id, output, .. } = &mut response {
            if damage == "response_call" { *call_id = "foreign".into(); }
            if damage == "pid" { output["execution"]["local_stop"]["scope"]["leader_pid"] = json!(0); }
        }
        assert!(!terminal_response_has_current_local_stop(&response, Some(&expected)), "{damage}");
        assert!(!terminal_response_proves_local_stop(&response, false, Some(&expected)), "{damage}");
        assert!(!terminal_response_proves_local_stop(&response, true, None), "a missing current identity cannot use Investigate fallback: {damage}");
    }
    let expected = identity();
    assert!(!terminal_response_proves_local_stop(&response(expected.clone(), ExecutionState::Deferred), false, Some(&expected)));
    assert!(!terminal_response_proves_local_stop(&response(expected.clone(), ExecutionState::NotStarted), false, Some(&expected)));
    let no_proof = HostResponse::ToolCompleted {
        call_id:"stop-call".into(), output:serde_json::to_value(ToolResult {
            contract:ToolResultContract::V1, data:Some(json!({"local_stop":identity(),"process_tree_stop":"confirmed"})),
            presentation:ToolPresentation{text:"Stopped".into(),summary:"Done".into()},
            execution:Some(ExecutionReceipt {state:ExecutionState::Unknown,..Default::default()}),
            full_output_ref:None,memory_usage:None,
        }).unwrap(), outcome:HostToolOutcome::Failed,activation:None,full_output_ref:None,artifacts:Vec::new(),
    };
    assert!(!terminal_response_proves_local_stop(&no_proof, false, Some(&expected)));
}

fn request(driver: &CliAgentHostDriver) -> HostRequestEnvelope {
    serde_json::from_value(json!({
        "request_id":"request-fenced","effect_id":"effect-fenced","host_id":"delivery-host",
        "session_id":"session-fenced","turn_id":"turn-fenced", "created_at_ms":0,
        "catalog_revision":driver.catalog.revision,"availability_epoch":driver.catalog.availability_epoch,
        "delivery":{"attempt_id":"attempt-fenced","worker_generation":"worker","lease_owner":"owner",
            "lease_token":"token","lease_expires_at_ms":i64::MAX},
        "payload":HostRequest::ExecuteTool {
            capability_id:"stop_fixture".into(),call_id:"stop-call".into(),input:json!({}),grant:None,
            permission_instructions:Vec::new(),permission_profile:nola_domain::PermissionProfile::Cautious,
        },
    })).unwrap()
}

fn driver(root: &std::path::Path) -> CliAgentHostDriver {
    let mut driver = CliAgentHostDriver::new("delivery-host", test_llm("http://127.0.0.1:1/v1".into()), test_context(root))
        .with_runtime_tool_ids(["stop_fixture".to_string()]);
    let mut capability = driver.catalog.capabilities["read_file"].clone();
    capability.id = "stop_fixture".into();
    capability.input_schema = json!({"type":"object"});
    capability.input_normalization = Default::default();
    capability.input_normalization.origin.kind = nola_core::agent::host::HostCapabilityOriginKind::Builtin;
    capability.input_normalization.effect = nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
        nola_core::agent::host::HostCapabilityEffect::Mutate);
    driver.catalog.capabilities.insert(capability.id.clone(), capability);
    driver
}

#[test]
fn admitted_not_started_requires_exact_control_owner_delivery_and_valid_source() {
    use nola_domain::host_effect::HostEffectId;
    use nola_domain::cancellation_not_started::{CancellationTargetNotStarted, CancellationRevokedAttempt};
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let _database = Database::bind(root.path());
    let mut driver = driver(root.path());
    driver.boundary_session_id = Some("conversation".into());
    driver.bound_dispatch_run_id = Some("turn-fenced".into());
    let mut control = request(&driver);
    let proof = CancellationTargetNotStarted {
        conversation_id:"conversation".into(),run_id:"turn-fenced".into(),
        core_session_id:control.session_id.clone(),host_id:control.host_id.0.clone(),
        cancellation_effect_id:control.effect_id.0.clone(),cancellation_request_id:control.request_id.clone(),
        target_effect_id:"unstarted-target".into(),target_request_id:"unstarted-request".into(),
        target_payload_identity:"a".repeat(64),checkpoint_revision:3,
        revoked_attempts:vec![CancellationRevokedAttempt{attempt_id:"revoked-attempt".into(),revision:2}],
    };
    control.payload = HostRequest::Cancel { target_effect_id:HostEffectId("unstarted-target".into()),target_not_started:Some(proof) };
    assert!(driver.has_admitted_not_started_target(&control));
    assert!(driver.quiescent_effects.is_empty(), "the source is the committed revocation, not a local cache");
    for damage in ["conversation","run","session","host","control","request","target","digest","revision","attempt","delivery","catalog","epoch"] {
        let mut foreign = control.clone();
        let HostRequest::Cancel { target_not_started:Some(proof),.. } = &mut foreign.payload else {unreachable!()};
        match damage {
            "conversation" => proof.conversation_id="another-conversation".into(),
            "run" => proof.run_id="another-run".into(),
            "session" => proof.core_session_id="another-session".into(),
            "host" => {proof.host_id="another-host".into();foreign.host_id.0="another-host".into();},
            "control" => proof.cancellation_effect_id="another-control".into(),
            "request" => proof.cancellation_request_id="another-request".into(),
            "target" => proof.target_effect_id="another-target".into(),
            "digest" => proof.target_payload_identity="not-a-digest".into(),
            "revision" => proof.checkpoint_revision=0,
            "attempt" => proof.revoked_attempts[0].attempt_id=" malformed ".into(),
            "delivery" => foreign.delivery=None,
            "catalog" => foreign.catalog_revision+=1,
            "epoch" => foreign.availability_epoch+=1,
            _=>unreachable!(),
        }
        assert!(!driver.has_admitted_not_started_target(&foreign),"{damage}");
    }
    let HostRequest::Cancel {target_not_started,..}=&mut control.payload else {unreachable!()};
    *target_not_started=None;
    assert!(!driver.has_admitted_not_started_target(&control),"a cold empty cache is not proof of no physical execution");
}

#[test]
fn dispatch_nonce_is_private_nested_scoped_and_old_stop_does_not_authorize_the_next_invocation() {
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let _database = Database::bind(root.path());
    let outer = identity();
    let _outer_guard = crate::tools::enter_local_execution_identity(outer.clone());
    let previous = std::sync::Arc::new(std::sync::Mutex::new(None::<LocalExecutionIdentity>));
    let capture = previous.clone();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler_calls = calls.clone();
    let parent = outer.clone();
    let mut driver = driver(root.path()).with_runtime_tool_handler(move |_, _, _, _, _| {
        handler_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let current = crate::tools::current_local_execution_identity().unwrap();
        assert_ne!(current.execution_id, parent.execution_id);
        assert!(std::thread::spawn(crate::tools::current_local_execution_identity).join().unwrap().is_none(),
            "unbound worker threads cannot manufacture a Host stop identity");
        let mut previous = capture.lock().unwrap();
        let proof = previous.clone().unwrap_or_else(|| current.clone());
        *previous = Some(current);
        Some(RuntimeToolResult { outcome:output(proof,ExecutionState::Unknown),task_promotion:None,task_update:None })
    });
    let mut target = request(&driver);
    driver.dispatch(target.clone(), &mut |_| Ok(())).unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let key = (target.host_id.0.clone(),target.session_id.clone(),target.turn_id.clone(),target.effect_id.0.clone());
    assert!(driver.quiescent_effects.contains_key(&key));
    assert_eq!(crate::tools::current_local_execution_identity(), Some(outer.clone()));
    let first = previous.lock().unwrap().clone().unwrap();
    let cached = driver.quiescent_effects.get(&key).unwrap();
    assert_eq!(cached.request_id, target.request_id);
    assert_eq!(cached.delivery_attempt_id.as_deref(), Some("attempt-fenced"));
    assert_eq!(cached.local_stop.as_ref().unwrap().identity, first);
    target.delivery.as_mut().unwrap().attempt_id = "next-attempt".into();
    driver.dispatch(target.clone(), &mut |_| Ok(())).unwrap();
    // The first mutation's failed work-step is durable. Redelivery cannot
    // repeat its handler just to obtain another physical-invocation nonce.
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(previous.lock().unwrap().as_ref(), Some(&first));
    assert!(!driver.quiescent_effects.contains_key(&key), "a prior invocation's proof must not survive a redispatch");
    assert_eq!(crate::tools::current_local_execution_identity(), Some(outer.clone()));

    // A genuinely new logical call may execute. The handler intentionally
    // returns the prior call's whole receipt, which must not stop this one.
    target.request_id = "new-invocation-request".into();
    target.effect_id.0 = "new-invocation-effect".into();
    target.delivery.as_mut().unwrap().attempt_id = "new-invocation-attempt".into();
    let HostRequest::ExecuteTool { call_id, .. } = &mut target.payload else { unreachable!() };
    *call_id = "new-invocation-call".into();
    let new_key = (target.host_id.0.clone(),target.session_id.clone(),target.turn_id.clone(),target.effect_id.0.clone());
    driver.dispatch(target.clone(), &mut |_| Ok(())).unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let current = previous.lock().unwrap().clone().unwrap();
    assert_ne!(current.execution_id, first.execution_id);
    assert_eq!(current.request_id, target.request_id);
    assert_eq!(current.effect_id, target.effect_id.0);
    assert_eq!(current.call_id, "new-invocation-call");
    assert_eq!(current.delivery_attempt_id.as_deref(), Some("new-invocation-attempt"));
    assert!(!driver.quiescent_effects.contains_key(&new_key), "an old invocation's receipt cannot stop a new call");
    assert!(!driver.quiescent_effects.contains_key(&key));
    assert_eq!(crate::tools::current_local_execution_identity(), Some(outer));
}

#[test]
fn isolated_batch_retires_old_stop_before_progress_delivery_or_worker_start() {
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let _database = Database::bind(root.path());
    let mut driver = driver(root.path());
    driver.catalog.capabilities.get_mut("read_file").unwrap().parallelism =
        HostCapabilityParallelism::ConcurrentSafe;
    let mut first = request(&driver);
    first.delivery = None;
    first.batch = Some(nola_host_protocol::HostRequestBatchIdentity {
        batch_id: "isolated-stop-invalidation".into(), index: 0, size: 2,
    });
    first.payload = HostRequest::ExecuteTool {
        capability_id: "read_file".into(), call_id: "stop-call".into(),
        input: json!({"path":root.path().join("never-opened.txt")}), grant: None,
        permission_instructions: Vec::new(), permission_profile: nola_domain::PermissionProfile::Cautious,
    };
    let key = (first.host_id.0.clone(),first.session_id.clone(),first.turn_id.clone(),first.effect_id.0.clone());
    let mut old_identity = identity();
    old_identity.delivery_attempt_id = None;
    driver.quiescent_effects.insert(key.clone(), ObservedHostStop::from_request(&first,
        Some(LocalExecutionStop {identity:old_identity,
            scope:LocalExecutionStopScope::OwnedProcessGroup {leader_pid:1234}})));
    assert!(driver.quiescent_effects.get(&key).unwrap().matches_owner(&key));
    let mut second = first.clone();
    second.request_id = "second-isolated-request".into();
    second.effect_id.0 = "second-isolated-effect".into();
    second.batch.as_mut().unwrap().index = 1;
    let mut progress = 0;
    let error = driver.dispatch_isolated_tool_batch(vec![first, second], &mut |response| {
        assert!(matches!(response.payload, HostResponse::ToolCallProgress {..}));
        progress += 1;
        Err("fixture rejects progress before workers start".into())
    }).unwrap_err();
    assert!(error.contains("fixture rejects progress"));
    assert_eq!(progress, 1, "the isolated branch, not sequential fallback, must run");
    assert!(!driver.quiescent_effects.contains_key(&key),
        "an isolated redispatch must retire the whole previous invocation witness even before execution");
    assert!(crate::tools::current_local_execution_identity().is_none());
}

#[test]
fn current_owned_stop_survives_response_rejection_but_still_needs_its_formal_cancel() {
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let _database = Database::bind(root.path());
    let mut driver = driver(root.path()).with_runtime_tool_handler(|_, _, _, _, _| {
        Some(RuntimeToolResult { outcome:output(crate::tools::current_local_execution_identity().unwrap(),ExecutionState::Unknown),
            task_promotion:None,task_update:None })
    });
    let target = request(&driver);
    assert!(driver.dispatch(target.clone(), &mut |response| {
        if matches!(response.payload,HostResponse::ToolCompleted {..}) { Err("fixture rejected business response".into()) }
        else { Ok(()) }
    }).unwrap_err().contains("fixture rejected business response"));
    assert!(crate::tools::current_local_execution_identity().is_none());
    let mut cancel = target.clone();
    cancel.effect_id.0 = "cancel-effect".into(); cancel.request_id = "cancel-request".into();
    cancel.delivery.as_mut().unwrap().attempt_id = "cancel-attempt".into();
    cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id, target_not_started: None };
    let mut confirmed = 0;
    driver.dispatch(cancel,&mut |response| {
        if matches!(response.payload,HostResponse::CancellationConfirmed {..}) { confirmed += 1; }
        Ok(())
    }).unwrap();
    assert_eq!(confirmed,1,"local stop alone does not replace an independent control");
}

#[test]
fn mcp_and_ownerless_requests_never_receive_builtin_local_execution_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut driver = driver(root.path());
    let mut target = request(&driver);
    let first = driver.fresh_local_execution_identity(&target).unwrap();
    assert_eq!(first.delivery_attempt_id.as_deref(),Some("attempt-fenced"));
    target.delivery = None;
    assert!(driver.fresh_local_execution_identity(&target).unwrap().delivery_attempt_id.is_none());
    target.turn_id = None;
    assert!(driver.fresh_local_execution_identity(&target).is_none());
    target.turn_id = Some("turn-fenced".into());
    driver.catalog.capabilities.get_mut("stop_fixture").unwrap().input_normalization.origin.kind =
        nola_core::agent::host::HostCapabilityOriginKind::Mcp;
    assert!(driver.fresh_local_execution_identity(&target).is_none());
}
