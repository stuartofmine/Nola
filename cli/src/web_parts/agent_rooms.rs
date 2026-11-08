// SPDX-License-Identifier: Apache-2.0
// The outer Web router owns authentication and CSRF. All room writes remain
// Application commands with revision checks and the single-writer journal.
// Only the current custom catalog is available to this Host. Apple built-ins
// are supplied as typed definitions by Apple; a missing custom ID never stands
// in for a built-in definition in the Web factory.
fn expert_prompt_templates_response() -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        application.execute_expert_host_operation(&uuid::Uuid::new_v4().to_string(),
            HostExpertTemplateOperation::ReadableTemplates { authority: expert_host_authority() })
            .map_err(|error| error.to_string())
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(422), json!({"code":"invalid_expert_prompt_catalog"})),
    }
}


fn agent_room_response(request: &mut Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    match execute_public_room_command(body, config) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"room_command_unconfirmed",
            "error":"Room operation could not be confirmed. Refresh the room before retrying."})),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MobileRoomCommand { expected_host_id: String, command: Value }

fn mobile_agent_room_response(request: &mut Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else { return json_response(StatusCode(401), json!({"error":"unauthorized"})); };
    let body = match read_mobile_json_value(request, &token) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let input: MobileRoomCommand = serde_json::from_value(body).map_err(|_| "invalid mobile room command")?;
        if input.expected_host_id != host_identity() { return Err("room Host identity changed".into()); }
        execute_public_room_command(input.command, config).map(|result| json!({"host_id":host_identity(),"result":result}))
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(_) => mobile_encrypted_json_response(StatusCode(409), json!({"code":"room_command_unconfirmed", "error":"Room command could not be confirmed"}), &token),
    }
}

fn public_room_command(body: Value) -> Result<nola_application::agent_room::AgentRoomCommand, String> {
    let command: nola_application::agent_room::AgentRoomCommand = serde_json::from_value(body).map_err(|_| "invalid room command")?;
    use nola_application::agent_room::AgentRoomCommand;
    if !matches!(&command, AgentRoomCommand::ForConversation { .. } | AgentRoomCommand::EnsureForConversation { .. }
        | AgentRoomCommand::AttachConversation { .. } | AgentRoomCommand::Directory | AgentRoomCommand::List { .. } | AgentRoomCommand::Read { .. }
        | AgentRoomCommand::ConfigureAgent { .. } | AgentRoomCommand::Create { .. } | AgentRoomCommand::Join { .. }
        | AgentRoomCommand::Deactivate { .. } | AgentRoomCommand::Submit { .. } | AgentRoomCommand::Submission { .. } | AgentRoomCommand::Submissions { .. } | AgentRoomCommand::CancelPending { .. }) {
        return Err("room command is owned by the Host".into());
    }
    Ok(command)
}

fn execute_public_room_command(body: Value, config: &Config) -> Result<Value, String> {
    use nola_application::agent_room::AgentRoomCommand;
    let command = public_room_command(body)?;
    let submission = matches!(&command, AgentRoomCommand::Submit { .. });
    let observations = matches!(&command, AgentRoomCommand::Submissions { .. });
    let exact_observation = matches!(&command, AgentRoomCommand::Submission { .. });
    let cancellation = match &command { AgentRoomCommand::CancelPending { room_id, submission_id, .. } => Some(format!("{room_id}/{submission_id}")), _ => None };
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Value>(
            nola_application::ApplicationCommand::AgentRoom { command }));
    result.and_then(|mut value| {
            if observations || exact_observation {
                let mut observe = |command| crate::application_port::CliApplicationPort::persistent_default()
                    .and_then(|mut app| app.execute::<Value>(command));
                if observations {
                    if let Some(values) = value.as_array_mut() {
                        for value in values { reconcile_observed_room_receipt(value, &mut observe)?; }
                    }
                } else { reconcile_observed_room_receipt(&mut value, &mut observe)?; }
            }
            if observations {
                if let Some(values) = value.as_array() { for value in values {
                    if let Ok(pending) = serde_json::from_value::<nola_application::agent_room::RoomSubmission>(value.clone()) {
                        start_room_dispatch(config.clone(),pending);
                    }
                } }
            }
            if let Some(key) = cancellation {
                if let Ok(active) = room_route_cancellations().lock() { if let Some(context) = active.get(&key) { context.request(); } }
            }
            if submission {
                if let Ok(accepted) = serde_json::from_value::<nola_application::agent_room::RoomSubmission>(value.clone()) {
                    start_room_dispatch(config.clone(), accepted);
                }
            }
            Ok(value)
    })
}

/// Materialize only a previously admitted execution's exact canonical seal.
/// An exact receipt read never authorizes dispatch or a replacement Run.
fn reconcile_observed_room_receipt(
    value: &mut Value,
    observe: &mut impl FnMut(nola_application::ApplicationCommand) -> Result<Value, String>,
) -> Result<(), String> {
    use nola_application::{ApplicationCommand, agent_room::{AgentRoomCommand, RoomSubmission, RoomSubmissionState}};
    if value.is_null() { return Ok(()); }
    let receipt: RoomSubmission = serde_json::from_value(value.clone()).map_err(|_| "invalid room receipt")?;
    if receipt.state != RoomSubmissionState::Executing { return Ok(()); }
    let run_id = receipt.execution_id.ok_or("room execution owner missing")?;
    let context = receipt.context_conversation_id.ok_or("room execution context missing")?;
    // Claim-before-Begin and a still-running delivery have no seal yet. This
    // exact read is not permission to dispatch or reject either pending owner.
    let terminal = observe(ApplicationCommand::CanonicalRunTerminal {
        conversation_id: context, run_id: run_id.clone(),
    })?;
    if terminal.is_null() { return Ok(()); }
    *value = observe(ApplicationCommand::AgentRoom { command: AgentRoomCommand::ObserveExecution {
        room_id: receipt.input.room_id, submission_id: receipt.input.submission_id,
        run_id, at_ms: now_unix_millis(),
    }})?;
    Ok(())
}

#[cfg(test)]
mod room_http_contract_tests {
    use super::*;
    #[test]
    fn exact_receipt_read_materializes_actual_store_seal_without_redispatch() {
        use nola_application::{agent_room as rooms, persistent_agent as agents, ApplicationCommand, BeginAssistantTurn};
        use nola_domain::persistent_agent::{AgentAvailability, AgentDefaults};
        use nola_core::agent::{host::*, protocol::*};
        use crate::application_port::CliApplicationPort;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("room-read.sqlite");
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        store.upsert_model_profile(&nola_store_port::model::StoredModelProfile {
            id:"fixture-model".into(),display_name:"Fixture".into(),provider:"fixture".into(),model_name:"fixture".into(),
            runtime:"http".into(),source_json:"{}".into(),parameters_json:"{}".into(),selected:true,enabled:true,
            revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
        }).unwrap();
        store.begin_write_unit_of_work().unwrap();
        agents::configure(&mut store.product_setting_repository(),agents::ConfigureAgent {
            id:"expert".to_string().try_into().unwrap(),expected_directory_revision:None,expected_agent_revision:None,
            name:"Expert".into(),description:"Receipt fixture".into(),defaults:AgentDefaults::try_from(json!({})).unwrap(),
            availability:AgentAvailability::Active,updated_at_ms:1,
        }).unwrap();
        let room=rooms::create(&mut store.product_setting_repository(),"Receipt fixture".into(),1).unwrap();
        let room=rooms::join(&mut store,rooms::JoinAgent {room_id:room.room.id().into(),expected_room_revision:room.revision,
            agent_id:"expert".to_string().try_into().unwrap(),expected_agent_revision:1,at_ms:2}).unwrap();
        let input=rooms::SubmitRoomMessage {room_id:room.room.id().into(),submission_id:uuid::Uuid::new_v4().to_string(),
            expected_room_revision:room.revision,text:"Real Store response".into(),participant_id:None,at_ms:3,
            parent_message_id:None,conversation_id:None};
        let accepted=rooms::submit(&mut store.product_setting_repository(),input.clone()).unwrap();
        let selected=rooms::select_recipient(&mut store.product_setting_repository(),&input.room_id,&input.submission_id,
            accepted.revision,&room.room.participants()[0].id,4).unwrap();
        let run_id=uuid::Uuid::new_v4().to_string();
        let selected=rooms::prepare_execution(&mut store,&input.room_id,&input.submission_id,selected.revision,
            rooms::RoomExecutionIntent {execution_id:run_id.clone(),envelope:rooms::RoomExecutionEnvelope::Native {
                platform:"android".into(),conversation_configuration_revision:1,model_profile_id:"fixture-model".into(),
                model_profile_revision:1,expected_parent_message_id:None,text:input.text.clone(),
            }},5).unwrap();
        let executing=rooms::claim_execution(&mut store,&input.room_id,&input.submission_id,selected.revision,6).unwrap().submission;
        let context=executing.context_conversation_id.clone().unwrap();
        store.commit_write_unit_of_work().unwrap();
        drop(store);
        let mut app=CliApplicationPort::InProcess(nola_application_bootstrap::open(&database).unwrap());
        let read=||ApplicationCommand::AgentRoom {command:rooms::AgentRoomCommand::Submission {
            room_id:input.room_id.clone(),submission_id:input.submission_id.clone()}};
        let mut not_started=app.execute::<Value>(read()).unwrap();
        let reserved=not_started.clone();
        reconcile_observed_room_receipt(&mut not_started,&mut |command|app.execute(command)).unwrap();
        assert_eq!(not_started,reserved,"Claim before Begin stays pending, without a factory or rejection");
        assert!(app.load_checkpoint(&context).unwrap().is_none());
        let descriptor=HostCapabilityDescriptor {id:"model.stream".into(),title:"Fixture".into(),route:HostCapabilityRoute::Model,
            availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Internal,permission_domain:None,
            input_schema:json!({"type":"object"}),input_normalization:Default::default(),
            output_policy:HostOutputPolicy {preview_char_limit:4000,persist_full_output:true,allow_artifacts:true},
            supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
            cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable};
        let begun=app.begin_assistant_turn(BeginAssistantTurn {
            conversation_id:context.clone(),expected_configuration_revision:Some(1),expected_parent_message_id:None,
            conversation_title:"Expert".into(),user_message_id:"user".into(),user_part_id:"user:text".into(),
            user_text:input.text.clone(),user_content_json:None,assistant_message_id:"assistant".into(),context_checkpoint_id:None,
            core_config:AgentCoreConfig {turn_identity:crate::test_model_identity::identity(),
                session_id:Some(SessionId::from_string(context.clone())),model:Some("fixture".into()),
                host_catalog:Some(HostCapabilityCatalog {host_id:HostId("fixture-host".into()),revision:1,availability_epoch:1,
                    lease:None,capabilities:[("model.stream".into(),descriptor)].into_iter().collect()}),..Default::default()},
            submission:AgentSubmission::UserInput {items:vec![AgentItem::user_message(ItemId::from_string("user".into()),&input.text)],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some(run_id.clone()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn},created_at_ms:1000,
        }).unwrap();
        let mut receipt=app.execute::<Value>(read()).unwrap();
        reconcile_observed_room_receipt(&mut receipt,&mut |command|app.execute(command)).unwrap();
        assert_eq!(receipt["state"],"executing","An active Run is not inferred complete from text or transport");
        assert!(receipt["terminal"].is_null());
        let still_running=receipt.clone();
        assert!(reconcile_observed_room_receipt(&mut receipt,&mut |_|Err("unavailable canonical read".into())).is_err());
        assert_eq!(receipt,still_running,"A failed read never manufactures a terminal outcome");
        let claimed=app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id:run_id.clone(),worker_generation:"fixture-generation".into(),lease_owner:"fixture-worker".into(),
            claimed_at_ms:1100,lease_expires_at_ms:61000,limit:1}).unwrap();
        let request=claimed.requests.into_iter().next().unwrap();
        let responses=[HostResponse::ExecutionStarted,HostResponse::ModelAttemptStarted {
            provider:"fixture-provider".into(),model:"fixture".into(),reason:"initial".into()},HostResponse::ModelCompleted {
                items:vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("answer".into()),"REAL_SEALED_REPLY")).unwrap()],
                tool_calls:vec![],usage:json!({})}];
        let mut sequence=request.resume_after_sequence.unwrap_or(0);
        for (index,payload) in responses.into_iter().enumerate() {
            sequence+=1;
            app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id:context.clone(),run_id:run_id.clone(),invocation_config_digest:begun.invocation_config_digest.clone(),
                response:HostResponseEnvelope {request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),
                    delegated_invocation_id:request.delegated_invocation_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),
                    lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),
                    catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,
                    delivery:request.delivery.clone(),sequence,payload},received_at_ms:1101+index as i64,
            }).unwrap();
        }
        // The direct factory has sealed the Run but has not observed the Room.
        let mut stale=app.execute::<Value>(read()).unwrap();
        assert_eq!(stale["state"],"executing");
        reconcile_observed_room_receipt(&mut stale,&mut |command|app.execute(command)).unwrap();
        assert_eq!(stale["state"],"completed");
        assert_eq!(stale["terminal"]["result"]["originating_submission_id"],run_id);
        assert_eq!(stale["terminal"]["result"]["value"]["kind"],"answered");
        let completed=stale.clone();
        let mut observations=0;
        reconcile_observed_room_receipt(&mut stale,&mut |_| {observations+=1;Err("must not redispatch or reobserve terminal".into())}).unwrap();
        assert_eq!(observations,0);assert_eq!(stale,completed);
        assert_eq!(app.execute::<Value>(read()).unwrap(),completed);
        let mut foreign=serde_json::to_value(executing).unwrap();
        foreign["context_conversation_id"]=json!("foreign-context");
        let unchanged=foreign.clone();
        reconcile_observed_room_receipt(&mut foreign,&mut |command|app.execute(command)).unwrap();
        assert_eq!(foreign,unchanged,"A foreign context cannot consume this Run's seal");
        assert!(app.execute::<Value>(ApplicationCommand::AgentRoom {command:rooms::AgentRoomCommand::ObserveExecution {
            room_id:input.room_id.clone(),submission_id:input.submission_id.clone(),run_id:"foreign-run".into(),at_ms:2000,
        }}).is_err());
        assert_eq!(app.execute::<Value>(read()).unwrap(),completed);
    }

    #[test]
    fn automatic_room_preference_uses_the_canonical_default_tier() {
        use nola_domain::reasoning_tiers::ReasoningTier;
        assert_eq!(room_reasoning_tier(&json!("automatic")).unwrap(),ReasoningTier::Balanced);
        assert_eq!(room_reasoning_tier(&Value::Null).unwrap(),ReasoningTier::Balanced);
        assert_eq!(room_reasoning_tier(&json!("detailed")).unwrap(),ReasoningTier::Detailed);
        assert!(room_reasoning_tier(&json!("invented")).is_err());
    }
    #[test]
    fn dispatch_reservation_never_steals_an_existing_owner_or_fails_durably_at_capacity() {
        let mut active = HashSet::new();
        assert!(reserve_room_dispatch(&mut active,"room/one",1));
        assert!(!reserve_room_dispatch(&mut active,"room/one",1));
        assert!(!reserve_room_dispatch(&mut active,"room/two",1));
        assert!(active.contains("room/one"));
        active.remove("room/one");
        assert!(reserve_room_dispatch(&mut active,"room/two",1));
    }
    #[test]
    fn reserved_full_group_child_uses_frozen_model_and_exact_configuration() {
        let entry:nola_domain::expert_message::ExpertMessageEntry=serde_json::from_value(json!({
            "participant":{"participant_id":"member","agent_id":"expert","agent_revision":2,"context_conversation_id":"child"},
            "display_name":"Expert","description":"","defaults":{},
            "model_declaration":{"profile_id":"frozen-profile","profile_revision":4,"provider":"original-provider","model_name":"original-model","runtime":"remote","parameters":{"temperature":0.1}},
            "context_configuration_revision":3,"child_submission_id":"child-run","child_user_message_id":"child-run:user","child_assistant_message_id":"child-run:assistant",
            "expected_prompt_definition":null,"model_witness":null,"instruction_witness":null
        })).unwrap();
        let profile:nola_application::ModelProfile=serde_json::from_value(json!({
            "id":"frozen-profile","display_name":"Original","provider":"original-provider","model_name":"original-model","runtime":"remote",
            "source_json":"{}","parameters_json":"{\"temperature\":0.1}","selected":false,"enabled":true,"revision":4,"created_at_ms":1,"updated_at_ms":1,"deleted_at_ms":null
        })).unwrap();
        assert!(validate_reserved_room_model(&entry,&profile,3).is_ok());
        assert!(validate_reserved_room_model(&entry,&profile,4).is_err());
        let mut current_default=profile.clone();current_default.id="current-default".into();
        assert!(validate_reserved_room_model(&entry,&current_default,3).is_err());
        let mut changed=profile.clone();changed.parameters_json="{\"temperature\":0.9}".into();
        assert!(validate_reserved_room_model(&entry,&changed,3).is_err());
        changed=profile;changed.revision+=1;
        assert!(validate_reserved_room_model(&entry,&changed,3).is_err());
    }
    #[test]
    fn public_transport_cannot_claim_or_assert_routing_or_terminal_state() {
        for command in [
            json!({"type":"claim_execution","room_id":"r","submission_id":"s","expected_revision":1,"at_ms":1}),
            json!({"type":"select_recipient","room_id":"r","submission_id":"s","expected_revision":1,"participant_id":"p","at_ms":1}),
            json!({"type":"reconcile_execution","room_id":"r","submission_id":"s","at_ms":1}),
            json!({"type":"execution_intent","room_id":"r","submission_id":"s"}),
            json!({"type":"observe_execution","room_id":"r","submission_id":"s","run_id":"run","at_ms":1}),
        ] { assert!(public_room_command(command).is_err()); }
        assert!(public_room_command(json!({"type":"submission","room_id":"r","submission_id":"s"})).is_ok());
        assert!(public_room_command(json!({"type":"submissions","room_id":"r","limit":100,"offset":0})).is_ok());
        assert!(public_room_command(json!({"type":"cancel_pending","room_id":"r","submission_id":"s","expected_revision":1,"at_ms":1})).is_ok());
        assert!(public_room_command(json!({"type":"for_conversation","conversation_id":"c"})).is_ok());
        assert!(public_room_command(json!({"type":"ensure_for_conversation","conversation_id":"c","at_ms":1})).is_ok());
        assert!(public_room_command(json!({"type":"attach_conversation","conversation_id":"c","room_id":"r","at_ms":1})).is_ok());
    }
}

fn validate_reserved_room_model(entry:&nola_domain::expert_message::ExpertMessageEntry,
    profile:&nola_application::ModelProfile,configuration_revision:i64)->Result<(),String> {
    let declaration=&entry.model_declaration;
    let parameters:Value=serde_json::from_str(&profile.parameters_json).map_err(|_|"room_invalid_model_parameters")?;
    if entry.context_configuration_revision!=configuration_revision || profile.id!=declaration.profile_id
        || profile.revision!=declaration.profile_revision || profile.provider!=declaration.provider
        || profile.model_name!=declaration.model_name || profile.runtime!=declaration.runtime
        || parameters!=declaration.parameters || !profile.enabled || profile.deleted_at_ms.is_some() {
        return Err("room_frozen_model_or_configuration_changed".into());
    }
    Ok(())
}

fn is_prepared_edited_room_child(context:Option<&str>)->Result<bool,String> {
    let Some(context)=context else{return Ok(false);};
    let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
    let prepared:Option<nola_domain::edited_resend::PreparedEditedResend>=app.execute(
        nola_application::ApplicationCommand::LoadPreparedEditedResend {conversation_id:context.into()})?;
    Ok(prepared.is_some())
}

fn start_room_dispatch(config: Config, accepted: nola_application::agent_room::RoomSubmission) {
    // Prepared children belong to their exact fresh admission, never this
    // ordinary settings-based background factory. Read failure grants no dispatch.
    if !matches!(is_prepared_edited_room_child(accepted.context_conversation_id.as_deref()),Ok(false)) {return;}

    // Reception returns before network/model work. The ledger inside route()
    // ensures a duplicate submission never authorizes another physical attempt.
    if !matches!(accepted.state, nola_application::agent_room::RoomSubmissionState::Accepted | nola_application::agent_room::RoomSubmissionState::Selected | nola_application::agent_room::RoomSubmissionState::Executing) { return; }
    let key = format!("{}/{}", accepted.input.room_id, accepted.input.submission_id);
    // Capacity is temporary backpressure, not a durable routing failure.
    // Both CLI recovery and HTTP observers use the same ownership guard.
    let Ok(Some(guard)) = RoomDispatchGuard::acquire(key) else { return; };
    let failed = accepted.clone();
    if thread::Builder::new().name("room-dispatch".into()).spawn(move || {
        let _guard = guard;
        match route_accepted_room(&config, accepted.clone()) {
            Ok(selected) => { let _ = execute_selected_room(&config, selected, None); },
            Err(code) => fail_room_dispatch(&accepted, if code == "room_routing_result_unknown" { "routing_result_unknown" }
                else if code.starts_with("room_routing_") { "routing_failed" } else { "routing_unavailable" }),
        }
    }).is_err() {
        fail_room_dispatch(&failed, "routing_unavailable");
    }
}

pub(crate) fn run_room_submission(config: &Config, accepted: nola_application::agent_room::RoomSubmission) -> Result<(), String> {
    run_room_submission_with_permission(config, accepted, None)
}

pub(crate) fn run_room_submission_with_permission(config: &Config, accepted: nola_application::agent_room::RoomSubmission,
    permission: Option<fn(&Value) -> Result<Value, String>>) -> Result<(), String> {
    let Some(_guard) = RoomDispatchGuard::acquire(format!("{}/{}", accepted.input.room_id, accepted.input.submission_id))? else { return Ok(()); };
    let routed = route_accepted_room(config, accepted.clone());
    #[cfg(any(feature = "terminal-ui", test))]
    if crate::interaction::current_execution_cancellation().is_some_and(|control|control.is_requested()) {
        use nola_application::{ApplicationCommand,agent_room::AgentRoomCommand};
        let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
        let current:nola_application::agent_room::RoomSubmission=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::Submit {input:accepted.input.clone()}})?;
        let _:Value=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::CancelPending {
            room_id:current.input.room_id,submission_id:current.input.submission_id,expected_revision:current.revision,at_ms:now_unix_millis(),
        }})?;
        return Ok(());
    }
    let selected=routed.map_err(|error| {fail_room_dispatch(&accepted,"routing_failed");error})?;
    execute_selected_room(config, selected, permission)
}

fn room_reasoning_tier(preference: &Value) -> Result<nola_domain::reasoning_tiers::ReasoningTier,String> {
    // Automatic is a conversation preference, not a fourth wire tier. The
    // Host's declared reasoning capability resolves the default tier later.
    if preference.is_null() || preference == "automatic" { return Ok(Default::default()); }
    serde_json::from_value(preference.clone()).map_err(|_|"invalid room reasoning preference".into())
}

fn room_debug_failure(config:&Config, message:&str) {
    if !config.debug_mode {return;}
    let redacted=crate::session::redact_sensitive_text(message);
    let safe=redacted.split_whitespace().map(|word|if word.contains("://") {"<endpoint>"}else{word})
        .collect::<Vec<_>>().join(" ");
    eprintln!("room execution diagnostic: {}",safe.chars().take(1000).collect::<String>());
}

fn room_stream_failure(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("error") { return None; }
    Some(event.get("message").or_else(||event.get("error")).and_then(Value::as_str)
        .filter(|message|!message.trim().is_empty()).unwrap_or("room_stream_failed").chars().take(4096).collect())
}

#[cfg(test)]
mod room_stream_error_tests {
    use super::*;
    #[test]
    fn exact_stream_failure_is_preserved_instead_of_reconciling_an_absent_run() {
        assert_eq!(room_stream_failure(&json!({"type":"error","message":"expert instruction capture changed"})),
            Some("expert instruction capture changed".into()));
        assert_eq!(room_stream_failure(&json!({"type":"message","content":"error"})),None);
        assert_eq!(room_stream_failure(&json!({"type":"error","message":""})),Some("room_stream_failed".into()));
    }
}

fn execute_selected_room(config: &Config, mut selected: nola_application::agent_room::RoomSubmission,
    permission: Option<fn(&Value) -> Result<Value, String>>) -> Result<(), String> {
    use nola_application::{ApplicationCommand, agent_room::{AgentRoomCommand, RoomExecutionClaim, RoomSubmissionState, RoomExecutionIntent, RoomExecutionEnvelope}};
    if !matches!(selected.state, RoomSubmissionState::Selected | RoomSubmissionState::Executing) { return Ok(()); }
    // An unavailable ownership read must not become a terminal routing failure.
    if is_prepared_edited_room_child(selected.context_conversation_id.as_deref())? {return Ok(());}
    let result = (|| -> Result<(), String> {
        let context_id = selected.context_conversation_id.clone().ok_or("room recipient context missing")?;
        let client_id = format!("room:{}", selected.input.room_id);
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let reserved=if let (Some(parent),Some(message))=(&selected.input.conversation_id,&selected.input.parent_message_id) {
            let manifest:Option<nola_domain::expert_message::ExpertMessageManifest>=app.execute_expert_host_operation(
                &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::LoadManifest {
                    conversation_id:parent.clone(),message_id:message.clone(),authority:expert_host_authority() })
                .map_err(|error| error.to_string())?;
            let manifest=manifest.ok_or("room_complete_expert_manifest_required")?;
            manifest.validate()?;
            let entry=manifest.entries.into_iter().find(|entry|entry.child_submission_id==selected.input.submission_id)
                .ok_or("room_reserved_child_missing")?;
            if entry.participant.context_conversation_id!=context_id
                || selected.selected_participant_id.as_deref()!=Some(entry.participant.participant_id.as_str()) {
                return Err("room_reserved_child_owner_changed".into());
            }
            Some(entry)
        } else {None};
        let persisted: Option<RoomExecutionIntent> = app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::ExecutionIntent {
            room_id:selected.input.room_id.clone(),submission_id:selected.input.submission_id.clone(),
        }})?;
        let intent = if let Some(intent) = persisted { intent } else {
            if selected.state != RoomSubmissionState::Selected { return Err("room execution intent missing".into()); }
            let tool_plan = host_tool_plan_prepare(HostToolPlanPrepare {
                expected_host_id: host_identity().into(), client_id: client_id.clone(), session_id: context_id.clone(),
            }, config)?;
            let (configuration_revision, settings) = conversation_configuration_state(&context_id)?.ok_or("room context configuration missing")?;
            let profile_id = if let Some(entry)=&reserved {entry.model_declaration.profile_id.as_str()} else {
                settings["modelSelectionID"].as_str().and_then(|id|id.strip_prefix("configured:")).ok_or("room model profile missing")?
            };
            let store = NolaStore::open_default()?;
            let profile = store.model_profile(profile_id)?.ok_or("room model profile missing")?;
            if let Some(entry)=&reserved { validate_reserved_room_model(entry,&profile,configuration_revision)?; }
            let frozen_settings=reserved.as_ref().map(|entry|entry.defaults.conversation_configuration()).unwrap_or(settings);
            let (_, messages) = load_store_conversation_messages(&store, &context_id).map_err(|_| "room context unavailable")?;
            let parent_id = messages.last().and_then(web_core_message_id);
            let execution_id = reserved.as_ref().map(|entry|entry.child_submission_id.clone())
                .unwrap_or_else(||uuid::Uuid::new_v4().to_string());
            let tier = room_reasoning_tier(&frozen_settings["modelOverrides"][format!("configured:{}",profile.id)]["reasoningPreference"])?;
            let mut body = json!({"expected_host_id":host_identity(),"client_id":client_id,"conversationId":context_id,
                "submission_id":execution_id,"user_message_id":format!("{execution_id}:user"),"assistant_message_id":format!("{execution_id}:assistant"),
                "expected_configuration_revision":configuration_revision,"expected_parent_message_id":parent_id,
                "execution_backend":"nola","tool_plan_ref":tool_plan.reference,
                "model_intent":{"profile_id":profile.id,"expected_revision":profile.revision,
                    "tier":tier,"overrides":{}},
                "messages":[{"role":"user","content":selected.input.text}]});
            body["request_digest"] = json!(nola_domain::remote_submission::submission_request_digest(&body)?);
            let intent = RoomExecutionIntent { execution_id, envelope:RoomExecutionEnvelope::Host {
                request:serde_json::from_value(body).map_err(|_|"invalid room execution request")?,tool_plan,
                model_profile_id:profile.id,model_profile_revision:profile.revision,
            }};
            selected = app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::PrepareExecution {
                room_id:selected.input.room_id.clone(),submission_id:selected.input.submission_id.clone(),expected_revision:selected.revision,
                intent:intent.clone(),at_ms:now_unix_millis(),
            }})?;
            intent
        };
        let claim: RoomExecutionClaim = app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::ClaimExecution {
            room_id: selected.input.room_id.clone(), submission_id: selected.input.submission_id.clone(),
            expected_revision: selected.revision, at_ms: now_unix_millis(),
        }})?;
        if claim.submission.state != RoomSubmissionState::Executing { return Ok(()); }
        if !claim.dispatch_permitted {
            // Ownership already exists or its outcome is unresolved. Do not
            // reinterpret a read/reconcile failure as permission for another attempt.
            let _=app.execute::<Value>(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ReconcileExecution {
                room_id:selected.input.room_id.clone(),submission_id:selected.input.submission_id.clone(),at_ms:now_unix_millis(),
            }});
            return Ok(());
        }
        let RoomExecutionEnvelope::Host { request, .. } = intent.envelope else { return Err("room execution belongs to a native owner".into()); };
        let prompt_definitions = if reserved.is_some() {
            let captured:Option<Vec<nola_domain::expert_message::ExpertPromptDefinition>> = app.execute_expert_host_operation(
                &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::LoadChildInstructions {
                    conversation_id:context_id.clone(),expected_run_id:selected.input.submission_id.clone(),authority:expert_host_authority(),
                }).map_err(|error|error.to_string())?;
            Some(captured.ok_or("room_captured_instruction_set_missing")?)
        } else {None};
        let body = serde_json::to_value(&request).map_err(|_|"invalid frozen room request")?;
        let identity = submission_wire_identity(&body,host_identity())?;
                // Admission reads the original source-issued plan from the
                // dedicated Store table. Never reinsert client/Room snapshots
                // or extend their authority during recovery.
                let response = chat_agent_stream_response_body_with_room_retry(body.clone(), config, None,
                    Some(RoomSourceAdmissionRetry {identity:identity.clone(),digest:request.request_digest.clone(),
                        prompt_definitions}));
                if response.status_code().0 != 200 {
                    let status = response.status_code().0;
                    let mut bytes = Vec::new();
                    let _ = response.into_reader().take(64 * 1024).read_to_end(&mut bytes);
                    let detail: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    // Log only a machine code, never provider bodies or credentials.
                    if matches!(detail["code"].as_str(),Some("source_admission_pending"|"source_admission_unknown")) {
                        return Ok(());
                    }
                    let code = detail["code"].as_str().filter(|code| code.len() <= 80
                        && code.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_'))
                        .unwrap_or("unclassified");
                    eprintln!("room admission rejected: status={status} code={code}");
                    // The handler's current preflight owner settles its own
                    // receipt. This observer must not reject another owner.
                } else {
        // Consume transport frames without accumulating output. Durable journals
        // remain the UI's source; a closed HTTP observer must not cancel a room.
                    let mut transport_failure = None;
                    if let Some(permission) = permission {
                        let monitor=RoomCliCancellationMonitor::start(&identity,&request.request_digest);
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(response.into_reader()).lines() {
                            let line = line.map_err(|error| error.to_string())?;
                            let Some(data) = line.strip_prefix("data: ") else { continue; };
                            let Ok(event) = serde_json::from_str::<Value>(data) else { continue; };
                            if let Some(error)=room_stream_failure(&event) { room_debug_failure(config,&error); transport_failure.get_or_insert(error); }
                            if event["type"] != "permission_request" { continue; }
                            if !room_permission_owned(&event, &context_id, &client_id, host_identity(), &request.submission_id) {
                                return Err("room permission owner mismatch".into());
                            }
                            let decision = serde_json::from_value(permission(&event)?)
                                .map_err(|_| "invalid room permission response")?;
                            submit_host_permission_decision(decision)?;
                        }
                        monitor.finish()?;
                    } else if config.debug_mode {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(response.into_reader()).lines() {
                            let line=line.map_err(|error|error.to_string())?;
                            let Some(data)=line.strip_prefix("data: ") else {continue;};
                            let Ok(event)=serde_json::from_str::<Value>(data) else {continue;};
                            if let Some(error)=room_stream_failure(&event) { room_debug_failure(config,&error); transport_failure.get_or_insert(error); }
                        }
                    } else {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(response.into_reader()).lines() {
                            let line=line.map_err(|error|error.to_string())?;
                            let Some(data)=line.strip_prefix("data: ") else {continue;};
                            let Ok(event)=serde_json::from_str::<Value>(data) else {continue;};
                            if let Some(error)=room_stream_failure(&event) { transport_failure.get_or_insert(error); }
                        }
                    }
                    if let Some(error)=transport_failure { return Err(error); }
                }
        let _: Value = app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::ReconcileExecution {
            room_id: selected.input.room_id.clone(), submission_id: selected.input.submission_id.clone(),
            at_ms: now_unix_millis(),
        }})?;
        Ok(())
    })();
    if let Err(error)=&result {room_debug_failure(config,error);fail_room_dispatch(&selected, "execution_unavailable");}
    result
}

// A CLI observer owns keyboard input, not the shared room. Forward cancellation
// through the exact source receipt; dropping an observer is never cancellation.
struct RoomCliCancellationMonitor {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<(),String>>>,
}
impl RoomCliCancellationMonitor {
    fn start(identity:&RemoteSubmissionIdentity,digest:&str)->Self {
        let stop=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(any(feature = "terminal-ui", test))]
        let worker=crate::interaction::current_execution_cancellation().map(|control| {
            let stopped=stop.clone();
            let body=json!({"identity":identity,"request_digest":digest});
            std::thread::spawn(move || {
                while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    if control.is_requested() {
                        let (status,result)=chat_agent_cancel_result(&body);
                        if status.0!=202 {return Err("Room cancellation was not confirmed; inspect the original execution".into());}
                        if result["cancelRequested"]!=true && result["terminationConfirmed"]!=true {
                            return Err("Room cancellation outcome is unknown; inspect the original execution".into());
                        }
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Ok(())
            })
        });
        #[cfg(not(any(feature = "terminal-ui", test)))]
        let worker = { let _ = (identity, digest); None };
        Self {stop,worker}
    }
    fn finish(mut self)->Result<(),String> {
        self.stop.store(true,std::sync::atomic::Ordering::SeqCst);
        self.worker.take().map(|worker|worker.join().map_err(|_|"Room cancellation observer failed".to_string())?).unwrap_or(Ok(()))
    }
}
impl Drop for RoomCliCancellationMonitor {
    fn drop(&mut self) {
        self.stop.store(true,std::sync::atomic::Ordering::SeqCst);
        if let Some(worker)=self.worker.take() {let _=worker.join();}
    }
}

fn room_permission_owned(event: &Value, context: &str, client: &str, host: &str, run: &str) -> bool {
    event["session_id"] == context && event["client_id"] == client && event["host_id"] == host
        && event["canonical_run_id"] == run
}

#[cfg(test)]
mod room_permission_owner_tests {
    use super::*;
    #[test]
    fn room_permission_requires_exact_context_client_host_and_run() {
        let value = json!({"session_id":"context","client_id":"room:one","host_id":"host","canonical_run_id":"run"});
        assert!(room_permission_owned(&value,"context","room:one","host","run"));
        for field in ["session_id","client_id","host_id","canonical_run_id"] {
            let mut changed = value.clone();
            changed[field] = json!("other");
            assert!(!room_permission_owned(&changed,"context","room:one","host","run"));
            changed.as_object_mut().unwrap().remove(field);
            assert!(!room_permission_owned(&changed,"context","room:one","host","run"));
        }
    }
}

fn room_dispatch_active() -> &'static Mutex<HashSet<String>> {
    static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}
struct RoomDispatchGuard(String);
impl RoomDispatchGuard {
    fn acquire(key: String) -> Result<Option<Self>,String> {
        let mut active = room_dispatch_active().lock().map_err(|_|"room dispatch registry unavailable")?;
        if !reserve_room_dispatch(&mut active,&key,8) { return Ok(None); }
        Ok(Some(Self(key)))
    }
}
fn reserve_room_dispatch(active: &mut HashSet<String>,key: &str,limit: usize) -> bool {
    if active.contains(key) || active.len() >= limit { return false; }
    active.insert(key.to_owned()); true
}
impl Drop for RoomDispatchGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = room_dispatch_active().lock() { active.remove(&self.0); }
        if let Ok(mut active) = room_route_cancellations().lock() { active.remove(&self.0); }
    }
}
fn room_route_cancellations() -> &'static Mutex<HashMap<String, crate::execution_context::ExecutionContext>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, crate::execution_context::ExecutionContext>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}
fn fail_room_dispatch(submission: &nola_application::agent_room::RoomSubmission, code: &str) {
    let _ = crate::application_port::CliApplicationPort::persistent_default().and_then(|mut app| app.execute::<Value>(
        nola_application::ApplicationCommand::AgentRoom {
            command: nola_application::agent_room::AgentRoomCommand::FailSubmission {
                room_id: submission.input.room_id.clone(), submission_id: submission.input.submission_id.clone(),
                expected_revision: submission.revision, code: code.into(), at_ms: now_unix_millis(),
            },
        }));
}

fn route_accepted_room(config: &Config, accepted: nola_application::agent_room::RoomSubmission)
    -> Result<nola_application::agent_room::RoomSubmission, String> {
    use nola_application::{ApplicationCommand, agent_room::{AgentRoomCommand, RoomSnapshot, RoomSubmissionState}};
    if accepted.state != RoomSubmissionState::Accepted { return Ok(accepted); }
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let room: Option<RoomSnapshot> = app.execute(ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Read { room_id: accepted.input.room_id.clone() } })?;
    let room = room.ok_or("room not found")?;
    let store = NolaStore::open_default()?;
    let (selection, _) = mobile_default_chat_model_projection(&store)?;
    if selection["kind"] != "resolved" { return Err("room coordinator model unavailable".into()); }
    let profile = store.model_profile(selection["profile_id"].as_str().ok_or("coordinator profile missing")?)?
        .ok_or("coordinator profile missing")?;
    if !mobile_model_profile_catalog(&store)?.iter().any(|value| value["profile_id"] == profile.id) {
        return Err("room coordinator model unavailable".into());
    }
    let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "invalid coordinator model source")?;
    let secret = crate::host_credentials::for_profile(&store, &profile, None)?;
    let mut settings = crate::model_profile_resolution::resolve_remote_profile_settings(&profile, &json!({}), &config.runtime,
        &[(source["baseURL"].as_str().ok_or("coordinator endpoint missing")?, secret.as_deref().unwrap_or(""))])?;
    crate::model_profile_resolution::bind_siwc_profile(&mut settings, &profile, &store)?;
    #[cfg(any(feature = "terminal-ui", test))]
    let cancellation = crate::interaction::current_execution_cancellation().map(|control|control.context()).unwrap_or_default();
    #[cfg(not(any(feature = "terminal-ui", test)))]
    let cancellation = crate::execution_context::ExecutionContext::default();
    room_route_cancellations().lock().map_err(|_| "room routing unavailable")?.insert(
        format!("{}/{}", accepted.input.room_id, accepted.input.submission_id), cancellation.clone());
    let llm = auxiliary_llm(settings, cancellation);
    let current: nola_application::agent_room::RoomSubmission = app.execute(nola_application::ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Submit { input:accepted.input.clone() },
    })?;
    if current.state != RoomSubmissionState::Accepted { return Ok(current); }
    crate::room_routing::route(&mut app, &room, &accepted, &llm, &profile.id, profile.revision.try_into().map_err(|_| "invalid model revision")?)
}
