// SPDX-License-Identifier: Apache-2.0
// Explicit user recovery on the source Host, never a generic Application RPC.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebRecoveryOwner {
    expected_host_id: String,
    client_id: String,
    conversation_id: String,
    run_id: String,
    assistant_message_id: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebRecoveryDecision {
    owner: WebRecoveryOwner,
    request_id: String,
    input: nola_protocol::ResolveUnknownEffectRecovery,
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum WebRecoveryRequest {
    Inspect { owner: WebRecoveryOwner },
    Receipt { decision: WebRecoveryDecision },
    Resolve { decision: WebRecoveryDecision },
}

fn valid_web_recovery_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.trim() == value && !value.chars().any(char::is_control)
}

fn validate_web_recovery_owner(
    application: &mut crate::application_port::CliApplicationPort,
    host: &str,
    owner: &WebRecoveryOwner,
) -> Result<(), String> {
    if [&owner.expected_host_id, &owner.client_id, &owner.conversation_id, &owner.run_id, &owner.assistant_message_id]
        .iter().any(|value| !valid_web_recovery_id(value)) || owner.expected_host_id != host {
        return Err("recovery source identity mismatch".into());
    }
    let admission = observation_command(application, json!({"type":"local_run_message_admission_head",
        "conversation_id":owner.conversation_id,"run_id":owner.run_id}))?;
    if admission.pointer("/identity/assistant_message_id").and_then(Value::as_str) != Some(owner.assistant_message_id.as_str()) {
        return Err("recovery requires the original source-local Run".into());
    }
    let receipt: Option<nola_domain::remote_submission::SourceSubmissionReceipt> = serde_json::from_value(
        observation_command(application, json!({"type":"observed_source_submission_receipt",
            "source_host_id":host,"client_id":owner.client_id,"conversation_id":owner.conversation_id,
            "run_id":owner.run_id,"assistant_message_id":owner.assistant_message_id}))?
    ).map_err(|error| error.to_string())?;
    let receipt = receipt.ok_or("recovery requires the original admitted browser client")?;
    receipt.validate()?;
    if receipt.phase != nola_domain::remote_submission::RemoteSubmissionPhase::Admitted {
        return Err("source submission has not admitted this Run".into());
    }
    Ok(())
}

impl WebRecoveryDecision {
    fn command_identity(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        self.input.validate()?;
        if !valid_web_recovery_id(&self.request_id) || self.input.conversation_id != self.owner.conversation_id
            || self.input.run_id != self.owner.run_id {
            return Err("recovery decision owner mismatch".into());
        }
        if matches!(&self.input.choice, nola_protocol::UnknownEffectRecoveryChoice::VerifyAppliedReceipt { .. }) {
            return Err("browser cannot supply Host execution receipts".into());
        }
        // Payload is NOT identity: changing an argument must reach Application's
        // same-request/different-payload conflict, never create another command.
        let identity = ("web-unknown-effect-recovery-v1", &self.owner, &self.request_id);
        Ok(format!("web-recovery:{:x}", Sha256::digest(serde_json::to_vec(&identity).map_err(|e| e.to_string())?)))
    }
}

fn web_recovery_receipt(
    application: &mut crate::application_port::CliApplicationPort,
    decision: &WebRecoveryDecision,
    submit: bool,
    task_owner: Option<&nola_application::owned_task_cancellation::OwnedTaskCancellation>,
) -> Result<Option<nola_protocol::CommandReceipt>, String> {
    let request_id = decision.command_identity()?;
    let command = match task_owner {
        Some(owner)=>nola_application::ApplicationCommand::ResolveOwnedTaskRecovery {owner:owner.clone(),input:decision.input.clone()},
        None=>nola_application::ApplicationCommand::ResolveUnknownEffectRecovery {input:decision.input.clone()},
    };
    match application {
        crate::application_port::CliApplicationPort::Service(client) => {
            let existing = client.command_receipt(&request_id)?;
            if existing.is_none() && !submit { return Ok(None); }
            // Existing admission atomically compares payload before replaying;
            // a bare receipt lookup cannot authenticate the caller's payload.
            let verified = client.submit_with_request_id(request_id.clone(), command)?;
            let canonical = client.command_receipt(&request_id)?;
            if canonical.as_ref() != Some(&verified) {
                return Err("recovery receipt payload was not verified".into());
            }
            Ok(canonical)
        }
        crate::application_port::CliApplicationPort::InProcess(runtime) => {
            web_recovery_in_process_receipt(runtime, &request_id, command, submit)
        }
        #[cfg(test)]
        crate::application_port::CliApplicationPort::ReceivedInProcess(runtime) => {
            web_recovery_in_process_receipt(runtime, &request_id, command, submit)
        }
    }
}

fn web_recovery_in_process_receipt(
    runtime: &mut nola_application_bootstrap::ApplicationRuntime,
    request_id: &str,
    command: nola_application::ApplicationCommand,
    submit: bool,
) -> Result<Option<nola_protocol::CommandReceipt>, String> {
    const CLIENT: &str = "web-unknown-effect-recovery";
    let existing = runtime.command_receipt(CLIENT, request_id).map_err(|error| error.to_string())?;
    if existing.is_none() && !submit { return Ok(None); }
    let mut args = serde_json::to_value(&command).map_err(|error| error.to_string())?;
    args.as_object_mut().ok_or("invalid recovery command")?.remove("type");
    runtime.execute_received_command(nola_application::ExecuteReceivedCommand {
        client_id: CLIENT.into(), request_id: request_id.into(), command_type: nola_protocol::CommandMetadata::wire_command_name(&command).into(),
        command_json: args.to_string(), expected_revision: None, received_at: chrono::Utc::now().to_rfc3339(),
    }, command).map(|value| Some(value.into_protocol_receipt())).map_err(|error| error.to_string())
}

fn execute_web_recovery(
    application: &mut crate::application_port::CliApplicationPort,
    host: &str,
    input: WebRecoveryRequest,
) -> Result<Value, String> {
    let owner = match &input {
        WebRecoveryRequest::Inspect { owner } => owner,
        WebRecoveryRequest::Receipt { decision } | WebRecoveryRequest::Resolve { decision } => &decision.owner,
    };
    validate_web_recovery_owner(application, host, owner)?;
    let result = match &input {
        WebRecoveryRequest::Inspect { owner } => {
            let inspection: Option<nola_protocol::UnknownEffectRecoveryInspection> = application.execute(
                nola_application::ApplicationCommand::InspectUnknownEffectRecovery {
                    conversation_id: owner.conversation_id.clone(), expected_run_id: owner.run_id.clone(),
                    assistant_message_id: owner.assistant_message_id.clone(),
                })?;
            json!({"inspection":inspection})
        }
        WebRecoveryRequest::Receipt { decision } | WebRecoveryRequest::Resolve { decision } => {
            let receipt = web_recovery_receipt(application, decision, matches!(&input, WebRecoveryRequest::Resolve { .. }), None)?;
            let receipt = receipt.map(|receipt| match receipt.outcome {
                nola_protocol::CommandReceiptOutcome::Accepted => json!({"status":"accepted"}),
                nola_protocol::CommandReceiptOutcome::Committed { result, .. } => json!({"status":"committed","result":result}),
                nola_protocol::CommandReceiptOutcome::Rejected { .. } => json!({"status":"rejected"}),
                nola_protocol::CommandReceiptOutcome::Conflict { .. } => json!({"status":"conflict"}),
            });
            json!({"request_id":decision.request_id,"receipt":receipt})
        }
    };
    Ok(json!({"source_host_id":host,"owner":owner,"result":result}))
}

fn web_unknown_effect_recovery_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    // Route inherits browser session/CSRF enforcement. This endpoint is not
    // on the public/mobile route allowlist and never accepts model-tool calls.
    let result = (|| {
        let mut bytes = Vec::new();
        request.as_reader().take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 8 * 1024 * 1024 { return Err("recovery request too large".into()); }
        let input = serde_json::from_slice::<WebRecoveryRequest>(&bytes).map_err(|e| e.to_string())?;
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        execute_web_recovery(&mut application, host_identity(), input)
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"recovery_unavailable","error":"Recovery is unavailable; retain the original decision and check again."})),
    }
}


fn web_owned_task_recovery_response(request:&mut Request)->Response<Box<dyn Read+Send>> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TaskOwner { host_id:String, conversation_id:String, parent_run_id:String, task_id:String }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Body {task:TaskOwner, recovery:Option<WebRecoveryRequest>}
    let result=(|| {
        let body:Body=serde_json::from_value(read_json_value(request).map_err(|_|"invalid task recovery request")?).map_err(|e|e.to_string())?;
        if body.task.host_id!=host_identity() { return Err("Task recovery Host changed".into()); }
        let task=nola_application::owned_task_cancellation::OwnedTaskCancellation {parent_conversation_id:body.task.conversation_id,
            parent_run_id:body.task.parent_run_id,task_id:body.task.task_id};
        let mut application=crate::application_port::CliApplicationPort::persistent_default()?;
        let Some(recovery)=body.recovery else {
            let inspection:Option<nola_protocol::UnknownEffectRecoveryInspection>=application.execute(
                nola_application::ApplicationCommand::InspectOwnedTaskRecovery {owner:task})?;
            return Ok(json!({"source_host_id":host_identity(),"inspection":inspection}));
        };
        let (decision,submit)=match recovery {
            WebRecoveryRequest::Receipt {decision}=>(decision,false),WebRecoveryRequest::Resolve {decision}=>(decision,true),
            WebRecoveryRequest::Inspect {owner}=> {
                if owner.expected_host_id!=host_identity() { return Err("Task recovery Host changed".into()); }
                let inspection:Option<nola_protocol::UnknownEffectRecoveryInspection>=application.execute(
                    nola_application::ApplicationCommand::InspectOwnedTaskRecovery {owner:task})?;
                if inspection.as_ref().is_some_and(|value|value.conversation_id!=owner.conversation_id
                    || value.run_id!=owner.run_id || value.assistant_message_id!=owner.assistant_message_id) {
                    return Err("Task recovery child changed".into());
                }
                return Ok(json!({"source_host_id":host_identity(),"owner":owner,"result":{"inspection":inspection}}));
            },
        };
        if decision.owner.expected_host_id!=host_identity()
            || !matches!(decision.input.choice,nola_protocol::UnknownEffectRecoveryChoice::KeepUnknownBlocked {..}) {
            return Err("Task recovery only preserves unknown cancellation".into());
        }
        let receipt=web_recovery_receipt(&mut application,&decision,submit,Some(&task))?;
        let receipt=receipt.map(|receipt|match receipt.outcome {
            nola_protocol::CommandReceiptOutcome::Accepted=>json!({"status":"accepted"}),
            nola_protocol::CommandReceiptOutcome::Committed {result,..}=>json!({"status":"committed","result":result}),
            nola_protocol::CommandReceiptOutcome::Rejected {..}=>json!({"status":"rejected"}),
            nola_protocol::CommandReceiptOutcome::Conflict {..}=>json!({"status":"conflict"}),
        });
        Ok::<_,String>(json!({"source_host_id":host_identity(),"owner":decision.owner,
            "result":{"request_id":decision.request_id,"receipt":receipt}}))
    })();
    match result {Ok(value)=>json_response(StatusCode(200),value),Err(_)=>json_response(StatusCode(409),
        json!({"code":"task_recovery_unavailable","error":"Task recovery is unavailable; retain the original decision and inspect the exact owner."}))}
}

#[cfg(test)]
mod web_unknown_effect_recovery_tests {
    use super::*;

    fn owner() -> WebRecoveryOwner {
        WebRecoveryOwner { expected_host_id: "source".into(), client_id: "browser".into(),
            conversation_id: "conversation".into(), run_id: "run".into(), assistant_message_id: "assistant".into() }
    }
    fn decision() -> WebRecoveryDecision {
        serde_json::from_value(json!({"owner":owner(),"request_id":"request", "input": {
            "conversation_id":"conversation","run_id":"run","invocation_config_digest":"digest","resolution_id":"resolution",
            "resolved_at_ms":2000,"targets":[{"effect_id":"effect","request_id":"effect-request","attempt_id":"attempt",
                "worker_generation":"worker","lease_owner":"owner","lease_token":"token","expected_attempt_revision":1}],
            "choice":{"kind":"authorize_risk_retry","reason":"Explicitly accept duplicate effects"}
        }})).unwrap()
    }

    #[test]
    fn recovery_wire_is_narrow_and_identity_excludes_payload_changes() {
        let decision = decision();
        let identity = decision.command_identity().unwrap();
        assert_eq!(identity, decision.clone().command_identity().unwrap());
        let mut changed = decision.clone(); changed.owner.client_id = "another-browser".into();
        assert_ne!(identity, changed.command_identity().unwrap());
        let mut changed = decision.clone(); changed.input.resolved_at_ms += 1;
        assert_eq!(identity, changed.command_identity().unwrap());
        let mut changed = decision.clone(); changed.input.targets[0].lease_token = "other-fence".into();
        assert_eq!(identity, changed.command_identity().unwrap());
        let mut changed = decision.clone(); changed.input.choice = nola_protocol::UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: "changed reason".into() };
        assert_eq!(identity, changed.command_identity().unwrap());
        let mut changed = decision.clone(); changed.input.choice = nola_protocol::UnknownEffectRecoveryChoice::VerifyAppliedReceipt {
            host_event_id: "forged".into(), response_json: "{}".into(),
        };
        assert!(changed.command_identity().is_err());
        assert!(serde_json::from_value::<WebRecoveryRequest>(json!({"operation":"resolve","decision":decision,"command":{"type":"begin_assistant_turn"}})).is_err());
        assert!(serde_json::from_value::<WebRecoveryRequest>(json!({"operation":"execute","command":{"type":"begin_assistant_turn"}})).is_err());
    }

    fn admit(application: &mut crate::application_port::CliApplicationPort) -> String {
        use nola_core::agent::{AgentCoreConfig, AgentSubmission, AgentItem, ItemId, SessionId, AgentInputDelivery};
        use nola_core::agent::host::*;
        let _: nola_domain::remote_submission::RegisteredSourceSubmission = application.execute(
            nola_application::ApplicationCommand::RegisterSourceSubmission {
                identity: nola_domain::remote_submission::RemoteSubmissionIdentity {
                    source_host_id: "source".into(), client_id: "browser".into(), conversation_id: "conversation".into(),
                    submission_id: "run".into(), user_message_id: "user".into(), assistant_message_id: "assistant".into(),
                }, request: json!({"input_items":[{"text":"Work"}]}),
            }).unwrap();
        let capability = HostCapabilityDescriptor {
            id:"model.stream".into(),title:"fixture".into(),route:HostCapabilityRoute::Model,
            availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Internal,
            permission_domain:None,input_schema:json!({"type":"object"}),input_normalization:Default::default(),
            output_policy:HostOutputPolicy {preview_char_limit:4000,persist_full_output:true,allow_artifacts:true},
            supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
            cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable,
        };
        application.begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id:"conversation".into(),conversation_title:"Recovery".into(),user_message_id:"user".into(),
            user_part_id:"user-body".into(),user_text:"Work".into(),user_content_json:None,assistant_message_id:"assistant".into(),context_checkpoint_id:None,
            core_config:AgentCoreConfig {turn_identity:crate::test_model_identity::identity(),session_id:Some(SessionId::from_string("conversation".into())),
                model:Some("fixture".into()),host_catalog:Some(HostCapabilityCatalog {host_id:HostId("source".into()),revision:1,
                    availability_epoch:1,lease:None,capabilities:[("model.stream".into(),capability)].into_iter().collect()}),..Default::default()},
            submission:AgentSubmission::UserInput {items:vec![AgentItem::user_message(ItemId::from_string("user".into()),"Work")],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some("run".into()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn},created_at_ms:1000,
        }).unwrap().invocation_config_digest
    }

    #[test]
    fn recovery_reads_require_exact_source_admission_and_original_browser_binding() {
        let mut application = crate::application_port::CliApplicationPort::ephemeral().unwrap();
        admit(&mut application);
        let result = execute_web_recovery(&mut application,"source",WebRecoveryRequest::Inspect {owner:owner()}).unwrap();
        assert_eq!(result["result"]["inspection"]["targets"], json!([]), "unclaimed work is not an unknown attempt");
        for field in ["expected_host_id", "client_id", "conversation_id", "run_id", "assistant_message_id"] {
            let mut wrong = serde_json::to_value(owner()).unwrap(); wrong[field] = "foreign".into();
            assert!(execute_web_recovery(&mut application,"source",WebRecoveryRequest::Inspect {
                owner:serde_json::from_value(wrong.clone()).unwrap() }).is_err(),"{field}");
            let mut d = decision(); d.owner = serde_json::from_value(wrong).unwrap();
            assert!(execute_web_recovery(&mut application,"source",WebRecoveryRequest::Receipt {decision:d}).is_err(),"receipt {field}");
        }
    }

    #[test]
    fn exact_web_decision_replays_real_application_receipt_without_another_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("recovery.sqlite");
        let mut application = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let digest = admit(&mut application);
        let mut decision = decision(); decision.input.invocation_config_digest = digest;
        let absent = execute_web_recovery(&mut application,"source",WebRecoveryRequest::Receipt {decision:decision.clone()}).unwrap();
        assert!(absent["result"]["receipt"].is_null());
        // Missing attempts are a genuine Application conflict, not fabricated
        // success. The received-command transaction still owns that outcome.
        let first = execute_web_recovery(&mut application,"source",WebRecoveryRequest::Resolve {decision:decision.clone()}).unwrap();
        assert_eq!(first["result"]["receipt"]["status"], "committed");
        assert_eq!(first["result"]["receipt"]["result"]["outcome"], "conflict");
        drop(application);
        let mut application = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let replay = execute_web_recovery(&mut application,"source",WebRecoveryRequest::Resolve {decision:decision.clone()}).unwrap();
        assert_eq!(first,replay);
        for variant in 0..3 {
            let mut changed = decision.clone();
            match variant {
                0 => changed.input.resolved_at_ms += 1,
                1 => changed.input.targets[0].lease_token = "different-fence".into(),
                _ => changed.input.choice = nola_protocol::UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason:"different reason".into() },
            }
            assert_eq!(decision.command_identity().unwrap(), changed.command_identity().unwrap());
            assert!(execute_web_recovery(&mut application,"source",WebRecoveryRequest::Receipt {decision:changed.clone()}).is_err());
            assert!(execute_web_recovery(&mut application,"source",WebRecoveryRequest::Resolve {decision:changed}).is_err());
        }
        let observed = execute_web_recovery(&mut application,"source",WebRecoveryRequest::Receipt {decision}).unwrap();
        assert_eq!(first,observed);
    }
}
