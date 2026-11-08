// SPDX-License-Identifier: Apache-2.0
//! Durable, secret-free room dispatch envelope. No recapture on recovery.
use super::*;
use nola_domain::host_tool_plan::{HostToolPlanReference, HostToolPlanSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomExecutionMessage { pub role: String, pub content: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomHostRequest {
    pub expected_host_id: String, pub client_id: String,
    #[serde(rename="conversationId")] pub conversation_id: String,
    pub submission_id: String, pub user_message_id: String, pub assistant_message_id: String,
    pub expected_configuration_revision: Option<i64>, pub expected_parent_message_id: Option<String>,
    pub execution_backend: String, pub tool_plan_ref: HostToolPlanReference,
    pub messages: Vec<RoomExecutionMessage>, pub request_digest: String,
    pub model_intent: nola_domain::run_generation_intent::RunGenerationIntent,
}

impl RoomHostRequest {
    /// The digest accompanies the wire request but is not part of its own
    /// canonical hash or the Store's frozen semantic request.
    pub fn semantic_request(&self) -> Result<serde_json::Value,String> {
        let mut body = serde_json::to_value(self).map_err(|_|"invalid room request")?;
        body.as_object_mut().ok_or("invalid room request")?.remove("request_digest");
        Ok(body)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="snake_case", deny_unknown_fields)]
pub enum RoomExecutionEnvelope {
    RemoteHost { identity: nola_domain::remote_submission::RemoteSubmissionIdentity, request_digest: String },
    Host { request: RoomHostRequest, tool_plan: HostToolPlanSnapshot, model_profile_id: String, model_profile_revision: i64 },
    Native { platform: String, conversation_configuration_revision: i64, model_profile_id: String,
        model_profile_revision: i64, expected_parent_message_id: Option<String>, text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomExecutionIntent { pub execution_id: String, pub envelope: RoomExecutionEnvelope }

fn intent_key(room: &str, submission: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(room).map_err(|_| "invalid room ID")?;
    uuid::Uuid::parse_str(submission).map_err(|_| "invalid submission ID")?;
    Ok(format!("agent_room_execution/{room}/{submission}"))
}

pub fn execution_intent<R: ProductSettingRepository + ?Sized>(store: &R, room: &str, submission: &str) -> Result<Option<RoomExecutionIntent>, String> {
    store.product_setting(&intent_key(room, submission)?).map_err(|e| e.to_string())?
        .map(|row| serde_json::from_str(&row.value_json).map_err(|_| "invalid room execution intent".into())).transpose()
}

pub fn prepare_execution<S>(store: &mut S, room: &str, submission: &str,
    expected_revision: i64, intent: RoomExecutionIntent, at_ms: i64) -> Result<RoomSubmission, String>
where S: ProductSettingRepositoryProvider + CommandProtocolRepositoryProvider + nola_store_port::remote_session::RemoteSessionRepository {
    if execution_intent(&store.product_setting_repository(),room,submission)?.is_none() {
        if let RoomExecutionEnvelope::RemoteHost { identity, request_digest } = &intent.envelope {
            identity.validate()?;
            let bound = store.remote_submission_intent(&identity.conversation_id).map_err(|e|e.to_string())?.ok_or("room_remote_request_not_bound")?;
            if bound.identity != *identity || bound.request_digest.as_deref() != Some(request_digest.as_str())
                || bound.phase != nola_domain::remote_submission::RemoteSubmissionPhase::Bound {
                return Err("room_remote_request_not_bound".into());
            }
            let body = store.remote_submission_request(identity,request_digest).map_err(|e|e.to_string())?;
            let row = store.product_setting_repository().product_setting(&format!("agent_room_submission/{room}/{submission}")).map_err(|e|e.to_string())?.ok_or("room_submission_not_found")?;
            let value: RoomSubmission = serde_json::from_str(&row.value_json).map_err(|_|"invalid room submission")?;
            if value.context_conversation_id.as_deref() != Some(identity.conversation_id.as_str())
                || identity.submission_id != intent.execution_id
                || body["messages"] != serde_json::json!([{"role":"user","content":value.input.text}]) {
                return Err("room_execution_intent_owner_mismatch".into());
            }
        }
    }
    store.command_protocol_repository().begin_command_mutations().map_err(|e|e.to_string())?;
    let result = prepare_execution_inner(&mut store.product_setting_repository(),room,submission,expected_revision,intent,at_ms);
    match result {
        Ok(value) => { store.command_protocol_repository().commit_command_mutations().map_err(|e|e.to_string())?; Ok(value) },
        Err(error) => { store.command_protocol_repository().rollback_command_mutations().map_err(|e|e.to_string())?; Err(error) },
    }
}

fn prepare_execution_inner<R: ProductSettingRepository + ?Sized>(store: &mut R, room: &str, submission: &str,
    expected_revision: i64, intent: RoomExecutionIntent, at_ms: i64) -> Result<RoomSubmission, String> {
    if !store.has_write_unit_of_work() || at_ms < 0 { return Err("room preparation requires admitted transaction".into()); }
    uuid::Uuid::parse_str(&intent.execution_id).map_err(|_| "invalid room execution ID")?;
    let key = format!("agent_room_submission/{room}/{submission}");
    let row = store.product_setting(&key).map_err(|e| e.to_string())?.ok_or("room_submission_not_found")?;
    let mut value: RoomSubmission = serde_json::from_str(&row.value_json).map_err(|_| "invalid room submission")?;
    if value.input.room_id != room || value.input.submission_id != submission { return Err("room_submission_identity_mismatch".into()); }
    if let Some(existing) = execution_intent(store, room, submission)? {
        return if existing == intent && value.execution_id.as_deref() == Some(intent.execution_id.as_str()) { Ok(value) } else { Err("room_execution_intent_conflict".into()) };
    }
    if value.revision != expected_revision || value.state != RoomSubmissionState::Selected { return Err("room_submission_not_preparable".into()); }
    let context = value.context_conversation_id.as_deref().ok_or("room_context_missing")?;
    match &intent.envelope {
        RoomExecutionEnvelope::RemoteHost { .. } => {},
        RoomExecutionEnvelope::Host { request, tool_plan, model_profile_id, model_profile_revision } => {
            if request.conversation_id != context || request.submission_id != intent.execution_id
                || request.client_id != format!("room:{room}") || request.user_message_id != format!("{}:user",intent.execution_id)
                || request.assistant_message_id != format!("{}:assistant",intent.execution_id) || request.execution_backend != "nola"
                || request.messages != vec![RoomExecutionMessage { role:"user".into(),content:value.input.text.clone() }]
                || request.tool_plan_ref != tool_plan.reference || model_profile_id.is_empty() || *model_profile_revision < 1
                || request.expected_configuration_revision.is_none_or(|revision|revision < 1)
                || request.model_intent.profile_id != *model_profile_id || request.model_intent.expected_revision != *model_profile_revision {
                return Err("room_execution_intent_owner_mismatch".into());
            }
            tool_plan.validate(&request.expected_host_id,&request.client_id,context,None)?;
            request.model_intent.validate()?;
            let body = request.semantic_request()?;
            if nola_domain::remote_submission::submission_request_digest(&body)? != request.request_digest { return Err("room_execution_digest_mismatch".into()); }
        },
        RoomExecutionEnvelope::Native { platform, conversation_configuration_revision, model_profile_id, model_profile_revision, text, .. } => {
            if !["ios","macos","android"].contains(&platform.as_str()) || *conversation_configuration_revision < 1
                || model_profile_id.is_empty() || *model_profile_revision < 1 || text != &value.input.text { return Err("invalid native room execution intent".into()); }
        },
    }
    let encoded = serde_json::to_string(&intent).map_err(|_| "invalid room execution intent")?;
    if encoded.len() > 2 * 1024 * 1024 { return Err("room execution intent exceeds budget".into()); }
    value.execution_id = Some(intent.execution_id);
    value.revision = value.revision.checked_add(1).ok_or("room_submission_revision_overflow")?;
    store.put_product_setting(&intent_key(room,submission)?, &encoded, at_ms).map_err(|e|e.to_string())?;
    store.put_product_setting(&key,&serde_json::to_string(&value).map_err(|e|e.to_string())?,at_ms).map_err(|e|e.to_string())?;
    Ok(value)
}

/// A failed transport is not a failed Run. Only a persisted source rejection
/// and absence of an admitted Run can close an execution admission failure.
pub fn reconcile_execution<S>(store: &mut S, room: &str, submission: &str, at_ms: i64) -> Result<RoomSubmission,String>
where S: ProductSettingRepositoryProvider + nola_store_port::remote_session::RemoteSessionRepository
    + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
    + nola_store_port::conversation_query::ConversationQueryRepository
    + nola_store_port::model::ModelRepositoryProvider {
    use nola_store_port::model::ModelRepository;
    let key = format!("agent_room_submission/{room}/{submission}");
    let row = store.product_setting_repository().product_setting(&key).map_err(|e|e.to_string())?.ok_or("room_submission_not_found")?;
    let mut value: RoomSubmission = serde_json::from_str(&row.value_json).map_err(|_|"invalid room submission")?;
    if value.input.room_id != room || value.input.submission_id != submission {
        return Err("room_execution_intent_identity_mismatch".into());
    }
    if value.state != RoomSubmissionState::Executing { return Ok(value); }
    let execution_id=value.execution_id.as_deref().ok_or("room_execution_owner_missing")?;
    if store.inspect_exact_run(execution_id).map_err(|e|e.to_string())?.is_some() {
        return observe_execution(store,room,submission,execution_id,at_ms);
    }
    let intent = execution_intent(&store.product_setting_repository(),room,submission)?.ok_or("room_execution_intent_required")?;
    if execution_id!=intent.execution_id {return Err("room_execution_intent_identity_mismatch".into());}
    let rejected = match intent.envelope {
        RoomExecutionEnvelope::RemoteHost { identity, request_digest } => {
            let accepted = store.exact_bound_remote_submission(&identity,&request_digest).map_err(|e|e.to_string())?;
            matches!(accepted.phase,nola_domain::remote_submission::RemoteSubmissionPhase::Rejected | nola_domain::remote_submission::RemoteSubmissionPhase::Cancelled)
        },
        RoomExecutionEnvelope::Native { conversation_configuration_revision,model_profile_id,model_profile_revision,expected_parent_message_id,.. } => {
            let snapshot = store.load_conversation_tail_snapshot(value.context_conversation_id.as_deref().ok_or("room_context_missing")?,1,1,1).map_err(|e|e.to_string())?;
            let profile = store.model_repository().model_profile(&model_profile_id).map_err(|e|e.to_string())?;
            snapshot.is_none_or(|snapshot| snapshot.conversation.configuration_revision != conversation_configuration_revision
                || snapshot.conversation.messages.last().map(|message|message.id.as_str()) != expected_parent_message_id.as_deref())
                || profile.is_none_or(|profile|profile.revision != model_profile_revision || !profile.enabled || profile.deleted_at_ms.is_some())
        },
        RoomExecutionEnvelope::Host { request, .. } => {
            let identity = nola_domain::remote_submission::RemoteSubmissionIdentity {
        source_host_id:request.expected_host_id, client_id:request.client_id,conversation_id:request.conversation_id,
        submission_id:request.submission_id,user_message_id:request.user_message_id,assistant_message_id:request.assistant_message_id,
            };
            store.source_submission_receipt(&identity,&request.request_digest).map_err(|e|e.to_string())?
                .is_some_and(|receipt| matches!(receipt.phase,nola_domain::remote_submission::RemoteSubmissionPhase::Rejected | nola_domain::remote_submission::RemoteSubmissionPhase::Cancelled))
        },
    };
    if rejected {
        let mut repository = store.product_setting_repository();
        if !repository.has_write_unit_of_work() || at_ms < 0 { return Err("room reconciliation requires admitted transaction".into()); }
        value.state = RoomSubmissionState::Failed;
        value.failure_code = Some("execution_unavailable".into());
        value.revision = value.revision.checked_add(1).ok_or("room_submission_revision_overflow")?;
        repository.put_product_setting(&key,&serde_json::to_string(&value).map_err(|e|e.to_string())?,at_ms).map_err(|e|e.to_string())?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_request_digest_is_not_part_of_its_semantic_request() {
        let mut wire=serde_json::json!({"expected_host_id":"host","client_id":"client","conversationId":"context",
            "submission_id":"execution","user_message_id":"user","assistant_message_id":"assistant",
            "expected_configuration_revision":1,"expected_parent_message_id":null,"execution_backend":"nola",
            "tool_plan_ref":{"manifest_id":"manifest","host_id":"host","client_id":"client","session_id":"context","catalog_revision":"catalog"},
            "messages":[{"role":"user","content":"hello"}],
            "model_intent":{"profile_id":"model","expected_revision":1,"tier":"balanced","overrides":{}}});
        let digest=nola_domain::remote_submission::submission_request_digest(&wire).unwrap();
        let semantic=wire.clone();
        wire["request_digest"]=serde_json::json!(digest);
        let request:RoomHostRequest=serde_json::from_value(wire).unwrap();
        assert_eq!(request.semantic_request().unwrap(),semantic);
        assert_eq!(nola_domain::remote_submission::submission_request_digest(&request.semantic_request().unwrap()).unwrap(),request.request_digest);
    }
    #[test]
    fn remote_intent_references_the_bound_request_without_copying_request_fields() {
        let value = serde_json::json!({"execution_id":"execution","envelope":{"kind":"remote_host",
            "identity":{"source_host_id":"host","client_id":"paired-client","conversation_id":"context",
                "submission_id":"execution","user_message_id":"independent-user","assistant_message_id":"independent-assistant"},
            "request_digest":"digest"}});
        assert!(serde_json::from_value::<RoomExecutionIntent>(value.clone()).is_ok());
        let mut invalid=value;
        invalid["envelope"]["request"]=serde_json::json!({"api_key":"not permitted"});
        assert!(serde_json::from_value::<RoomExecutionIntent>(invalid).is_err());
    }
    #[test]
    fn native_intent_has_no_untyped_credentials_or_configuration_channel() {
        let native = serde_json::json!({"execution_id":uuid::Uuid::new_v4().to_string(),"envelope":{
            "kind":"native","platform":"android","conversation_configuration_revision":3,"model_profile_id":"profile",
            "model_profile_revision":2,"expected_parent_message_id":null,"text":"search 当前 follow-up"}});
        assert!(serde_json::from_value::<RoomExecutionIntent>(native.clone()).is_ok());
        for field in ["api_key","headers","configuration","provider_request"] {
            let mut rejected=native.clone(); rejected["envelope"][field]=serde_json::json!("secret");
            assert!(serde_json::from_value::<RoomExecutionIntent>(rejected).is_err());
        }
    }
}
