// SPDX-License-Identifier: Apache-2.0
// Read-only observation of an Application-admitted execution, never a resume.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedWorkerOwner {
    conversation_id: String,
    run_id: String,
    assistant_message_id: String,
    client_id: String,
    generation: u64,
}
static OBSERVED_WORKERS: OnceLock<Mutex<HashMap<u64, ObservedWorkerOwner>>> = OnceLock::new();
static OBSERVED_PERMISSION_EVENTS: OnceLock<Mutex<HashMap<String, (ObservedWorkerOwner, Value)>>> = OnceLock::new();

fn observation_command(application: &mut crate::application_port::CliApplicationPort, body: Value) -> Result<Value, String> {
    application.execute(serde_json::from_value::<nola_application::ApplicationCommand>(body).map_err(|e| e.to_string())?)
}

fn bind_observed_worker(owner: &ObservedWorkerOwner) -> Result<(), String> {
    // Immutable execution ownership is admitted once, not reconstructed from
    // the cumulative checkpoint for every token notification.
    let cached = OBSERVED_WORKERS.get_or_init(||Mutex::new(HashMap::new())).lock()
        .map_err(|_|"worker identity unavailable")?.get(&owner.generation).cloned();
    if let Some(existing) = cached {
        let alive=active_web_agent_runs().lock().map_err(|_|"worker registry unavailable")?
            .get(&owner.conversation_id).is_some_and(|r|r.execution_id()==owner.generation);
        return if existing==*owner && alive {Ok(())} else {Err("execution owner changed".into())};
    }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let exact = observation_command(&mut application, json!({"type":"load_exact_run_invocation",
        "conversation_id":owner.conversation_id,"expected_run_id":owner.run_id,"assistant_message_id":owner.assistant_message_id}))?;
    if exact.get("core_session_id").and_then(Value::as_str).is_none() {
        return Err("Run is not yet admitted".into());
    }
    // A remote resume cannot give an ordinary source-local Run a new client.
    // The original source submission receipt is the durable client binding.
    let receipt = observation_command(&mut application, json!({
        "type":"observed_source_submission_receipt","source_host_id":host_identity(),
        "client_id":owner.client_id,"conversation_id":owner.conversation_id,
        "run_id":owner.run_id,"assistant_message_id":owner.assistant_message_id
    }))?;
    let receipt:Option<nola_domain::remote_submission::SourceSubmissionReceipt>=serde_json::from_value(receipt).map_err(|e|e.to_string())?;
    let receipt=receipt.ok_or("Run has no admitted remote client binding")?;
    receipt.validate()?;
    if receipt.phase != nola_domain::remote_submission::RemoteSubmissionPhase::Admitted {
        return Err("remote submission is not admitted".into());
    }
    let runs = active_web_agent_runs().lock().map_err(|_| "worker registry unavailable")?;
    if !runs.get(&owner.conversation_id).is_some_and(|run| run.execution_id() == owner.generation) {
        return Err("execution generation changed".into());
    }
    let mut bound = OBSERVED_WORKERS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(|_| "worker identity unavailable")?;
    if bound.get(&owner.generation).is_some_and(|old| old != owner) { return Err("execution owner changed".into()); }
    bound.insert(owner.generation, owner.clone());
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(tag="operation",rename_all="snake_case",deny_unknown_fields)]
enum RunObservationRequest {
    Discover { conversation_id: String, expected_host_id: String, client_id: String },
    Observe { conversation_id: String, expected_host_id: String, client_id: String, expected_run_id: String, assistant_message_id: String },
}

#[derive(Debug)]
struct RunObservationFailure {
    code: &'static str,
    message: String,
}
impl From<String> for RunObservationFailure {
    fn from(message: String) -> Self { Self { code: "observation_unavailable", message } }
}
impl From<&str> for RunObservationFailure {
    fn from(message: &str) -> Self { message.to_owned().into() }
}
impl RunObservationFailure {
    fn control(message: impl Into<String>) -> Self {
        Self { code: "run_control_unconfirmed", message: message.into() }
    }
}

fn run_observation_response(request: &mut Request, mobile: bool) -> Response<Box<dyn Read + Send>> {
    run_observation_response_with(request,mobile,||Ok((crate::application_port::CliApplicationPort::persistent_default()?,host_identity().to_owned())))
}

fn run_observation_response_with(request: &mut Request, mobile: bool,
    open: impl FnOnce()->Result<(crate::application_port::CliApplicationPort,String),String>
) -> Response<Box<dyn Read + Send>> {
    if request.body_length().is_some_and(|length|length>64*1024) {
        return json_response(StatusCode(413),json!({"error":"observation request exceeds budget"}));
    }
    let token = if mobile { match mobile_bearer_token(request) { Some(value) => Some(value), None => return json_response(StatusCode(401),json!({"error":"unauthorized"})) } } else { None };
    let result = (|| -> Result<Value, RunObservationFailure> {
        let mut bytes=Vec::new();
        request.as_reader().take(64*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>64*1024 {return Err("observation request exceeds budget".into());}
        if token.is_some() && request_header(request,"X-Nola-Mobile-Auth").is_some()
            && !nola_core::mobile_crypto::mobile_body_sha256(&bytes).eq_ignore_ascii_case(request_header(request,"X-Nola-Mobile-Body-SHA256").unwrap_or("")) {
            return Err("mobile request body authentication failed".into());
        }
        let raw:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
        let body=match token.as_deref() {Some(token)=>{
            let encrypted=serde_json::from_value::<nola_core::mobile_crypto::MobileEncryptedEnvelope>(raw).map_err(|e|e.to_string())?;
            nola_core::mobile_crypto::decrypt_mobile_json(&encrypted,token)?
        },None=>raw};
        let input = serde_json::from_value::<RunObservationRequest>(body).map_err(|e| e.to_string())?;
        let (mut application,host) = open()?;
        read_run_observation(&mut application, &host, input)
    })();
    let (status, value) = match result { Ok(value) => (StatusCode(200), value), Err(error) => (StatusCode(409),json!({"code":error.code,"error":error.message})) };
    match token { Some(token) => mobile_encrypted_json_response(status,value,&token), None => json_response(status,value) }
}

/// A child is readable through its immutable Core lineage. This grants no
/// browser submission, cancellation, recovery, or permission authority.
fn observed_task_execution(application: &mut crate::application_port::CliApplicationPort,
    conversation: &str, run: &str) -> Result<Option<nola_domain::ConversationExecutionOwnership>, String> {
    let window: nola_application::ConversationProjectionWindowV1 = application.execute(
        nola_application::ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id:conversation.into(), before_ordinal:None, limit:1, content_byte_limit:1024,
        })?;
    let owner=window.projection.execution_ownership;
    if let nola_domain::ConversationExecutionOwnership::TaskChild {ref task_id, ..}=owner {
        let binding:Option<nola_domain::task_execution::RuntimeTaskRunBinding>=application.execute(
            nola_application::ApplicationCommand::RuntimeTask { command:nola_application::RuntimeTaskCommand::RunBinding { task_id:task_id.clone() } })?;
        let binding=binding.ok_or("Child Run binding unavailable")?;
        if !task_observation_binding_matches(&owner, &binding, conversation, run) {
            return Err("Child Run observation binding mismatch".into());
        }
        Ok(Some(owner))
    } else { Ok(None) }
}

fn task_observation_binding_matches(owner: &nola_domain::ConversationExecutionOwnership,
    binding: &nola_domain::task_execution::RuntimeTaskRunBinding, conversation: &str, run: &str) -> bool {
    matches!(owner,nola_domain::ConversationExecutionOwnership::TaskChild {
        parent_conversation_id,task_id,child_conversation_id,..}
        if child_conversation_id == conversation && binding.child_session_id == conversation
            && binding.run_id == run && binding.owner_session_id == *parent_conversation_id && binding.task_id == *task_id)
}

#[cfg(test)]
mod task_observation_tests {
    use super::*;
    #[test]
    fn read_only_child_observation_requires_exact_immutable_binding() {
        let owner=nola_domain::ConversationExecutionOwnership::TaskChild{parent_conversation_id:"parent".into(),parent_run_id:"parent-run".into(),task_id:"task".into(),child_conversation_id:"child".into()};
        let binding=nola_domain::task_execution::RuntimeTaskRunBinding{task_id:"task".into(),owner_session_id:"parent".into(),child_session_id:"child".into(),run_id:"child-run".into(),admission_attempt_id:"attempt".into(),input_watermark:None,bound_at_ms:1};
        assert!(task_observation_binding_matches(&owner,&binding,"child","child-run"));
        assert!(!task_observation_binding_matches(&owner,&binding,"child","foreign-run"));
        assert!(!task_observation_binding_matches(&owner,&binding,"foreign-child","child-run"));
        let mut changed=binding.clone();changed.owner_session_id="foreign-parent".into();
        assert!(!task_observation_binding_matches(&owner,&changed,"child","child-run"));
        changed=binding.clone();changed.task_id="foreign-task".into();
        assert!(!task_observation_binding_matches(&owner,&changed,"child","child-run"));
        assert!(!task_observation_binding_matches(&nola_domain::ConversationExecutionOwnership::User,&binding,"child","child-run"));
    }
}

fn read_run_observation(application: &mut crate::application_port::CliApplicationPort, host: &str, input: RunObservationRequest) -> Result<Value,RunObservationFailure> {
    let (conversation, expected_host, client, requested) = match input {
        RunObservationRequest::Discover{conversation_id,expected_host_id,client_id} => (conversation_id,expected_host_id,client_id,None),
        RunObservationRequest::Observe{conversation_id,expected_host_id,client_id,expected_run_id,assistant_message_id} => (conversation_id,expected_host_id,client_id,Some((expected_run_id,assistant_message_id))),
    };
    if [&conversation,&expected_host,&client].iter().any(|s| s.trim().is_empty() || s.len()>512) || expected_host != host { return Err("observation identity mismatch".into()); }
    let goal_projection = crate::session::load_session_goal_projection(&conversation)?;
    let goal_control = goal_projection.as_ref().and_then(|projection| projection.control_observation());
    let active = observation_command(application,json!({"type":"active_run","conversation_id":conversation}))?;
    let generation = active_web_agent_runs().lock().map_err(|_| "worker registry unavailable")?.get(&conversation).map(|r|r.execution_id());
    let (run, assistant) = match requested {
        Some(ids) => ids,
        None if active.is_null() && (generation.is_none() || goal_control.is_some()) => return Ok(json!({"source_host_id":host,"conversation_id":conversation,"client_id":client,"goal_control":goal_control,"result":null})),
        None => (active.get("run_id").and_then(Value::as_str).ok_or("Run is starting")?.to_owned(),
            active.get("assistant_message_id").and_then(Value::as_str).ok_or("missing root assistant")?.to_owned()),
    };
    if [&run,&assistant].iter().any(|s|s.trim().is_empty() || s.len()>512) { return Err("invalid Run identity".into()); }
    // Source-local immutable admission gates imported/foreign Runs even when
    // this database happens to contain their replicated journal records.
    let admission = observation_command(application,json!({"type":"local_run_message_admission_head","conversation_id":conversation,"run_id":run}))?;
    if admission.pointer("/identity/assistant_message_id").and_then(Value::as_str) != Some(assistant.as_str()) { return Err("root assistant mismatch".into()); }
    let seal = observation_command(application,json!({"type":"local_assistant_run_terminal_seal","conversation_id":conversation,"run_id":run}))?;
    let goal_execution = goal_projection.as_ref().and_then(|projection| projection.execution_owner(&run));
    let goal_allows_submit = goal_execution.is_none() || goal_projection.as_ref().and_then(|p| p.active.as_ref())
        .is_some_and(|goal| goal.status.is_terminal());
    let bound = generation.and_then(|id| OBSERVED_WORKERS.get_or_init(||Mutex::new(HashMap::new())).lock().ok()?.get(&id).cloned());
    let matching = bound.as_ref().filter(|o|o.conversation_id==conversation && o.run_id==run && o.assistant_message_id==assistant && (o.client_id==client || (goal_execution.is_some() && o.client_id=="web-goal")));
    let state = match (generation, bound.as_ref(), matching) { (None,_,_)=>"absent",(Some(_),None,_)=>"starting",(_,_,Some(_))=>"running",_=>"unknown" };
    let recovery = if !seal.is_null() {"terminal"} else if state=="running" || state=="starting" {"observe"} else if state=="absent" {"resume_required"} else {"inconsistent"};
    let mut permissions = Vec::new();
    if let Some(owner) = matching.filter(|owner|active.get("run_id").and_then(Value::as_str)==Some(run.as_str()) && seal.is_null()
        && OBSERVED_PERMISSION_EVENTS.get_or_init(||Mutex::new(HashMap::new())).lock().is_ok_and(|events|events.values().any(|(bound,_)|bound==*owner))) {
        let projection = observation_command(application,json!({"type":"load_conversation_permission_projection_v1",
            "conversation_id":conversation,"run_id":run,"scope":{"type":"current_run"}}))?;
        if projection.get("conversation_id").and_then(Value::as_str)!=Some(conversation.as_str())
            || projection.get("run_id").and_then(Value::as_str)!=Some(run.as_str()) {
            return Err("permission projection belongs to another execution".into());
        }
        let events = OBSERVED_PERMISSION_EVENTS.get_or_init(||Mutex::new(HashMap::new())).lock().map_err(|_|"permission observation unavailable")?;
        let challenges = HOST_PERMISSION_CHALLENGES.get_or_init(||Mutex::new(HashMap::new())).lock().map_err(|_|"permission challenges unavailable")?;
        let waiters = HOST_PERMISSION_WAITERS.get_or_init(||Mutex::new(HashMap::new())).lock().map_err(|_|"permission waiters unavailable")?;
        for (nonce,(event_owner,event)) in events.iter() {
            if event_owner != owner || permissions.len()>=16 {continue;}
            let Some(challenge)=challenges.get(nonce) else {continue};
            if challenge.expires_at<=unix_time_seconds() || !waiters.contains_key(nonce) {continue;}
            if projection.pointer("/permissions/pending").and_then(Value::as_array).is_some_and(|pending| pending.iter().any(|p|
                p.get("request_id").and_then(Value::as_str)==Some(challenge.request_id.as_str())
                && p.get("turn_id")==projection.get("active_turn_id")
                && p.get("call_id")==event.get("call_id")
                && p.get("capability_id").and_then(Value::as_str)==Some(challenge.tool_name.as_str()))) {
                permissions.push(event.clone());
            }
        }
    }
    if let Some(owner) = matching.filter(|_| active.get("run_id").and_then(Value::as_str) == Some(run.as_str()) && seal.is_null()) {
        let projection = observation_command(application, json!({"type":"load_conversation_permission_projection_v1",
            "conversation_id":conversation,"run_id":run,"scope":{"type":"current_run"}}))?;
        if let Some(turn) = projection.get("active_turn_id").and_then(Value::as_str) {
            let remaining = 16usize.saturating_sub(permissions.len());
            permissions.extend(observed_delegated_permissions(owner, turn)?.into_iter().take(remaining));
        }
    }
    let latest = observation_command(application,json!({"type":"active_run","conversation_id":conversation}))?;
    let latest_generation=active_web_agent_runs().lock().map_err(|_|"worker registry unavailable")?.get(&conversation).map(|r|r.execution_id());
    if latest_generation!=generation || latest.get("run_id")!=active.get("run_id")
        || latest.get("assistant_message_id")!=active.get("assistant_message_id") {return Err("execution changed during observation".into());}
    let submission_receipt = observation_command(application, json!({
        "type":"observed_source_submission_receipt","source_host_id":host,"client_id":client,
        "conversation_id":conversation,"run_id":run,"assistant_message_id":assistant
    })).map_err(RunObservationFailure::control)?;
    let task_execution = if submission_receipt.is_null() && goal_execution.is_none() {
        observed_task_execution(application, &conversation, &run)?
    } else { None };
    if submission_receipt.is_null() && goal_execution.is_none() && task_execution.is_none() {
        return Err(RunObservationFailure::control("Run has no confirmed source submission owner for this client"));
    }
    if !submission_receipt.is_null() && goal_execution.is_some() {
        return Err("Run cannot have both remote source and local Goal ownership".into());
    }
    let context_usage = observation_command(application,json!({"type":"local_run_context_usage",
        "identity":admission.get("identity").ok_or("missing admission root")?}))?;
    let read_only=task_execution.is_some();
    if read_only { permissions.clear(); }
    let recovery=if read_only && seal.is_null() {"observe"} else {recovery};
    let response=json!({"source_host_id":host,"conversation_id":conversation,"client_id":client,"goal_control":goal_control,"result":{"protocol_version":1,"conversation_id":conversation,"client_id":client,"run_id":run,
        "assistant_message_id":assistant,"status":if seal.is_null() {"running"} else {"sealed"},
        "worker":{"state":state,"generation":generation.map(|g|g.to_string())},"recovery":recovery,
        "admission":admission,"submission_receipt":submission_receipt,"goal_execution":goal_execution,"task_execution":task_execution,"context_usage":context_usage,"terminal_seal":seal,"permissions":permissions,"can_submit":!read_only && goal_allows_submit && !seal.is_null() && latest.is_null() && latest_generation.is_none()}});
    if serde_json::to_vec(&response).map_err(|e|e.to_string())?.len()>256*1024 {return Err("observation response exceeds budget".into());}
    Ok(response)
}
