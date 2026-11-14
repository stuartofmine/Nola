// SPDX-License-Identifier: Apache-2.0
// Renderer expresses only an edited-send intent. Host owns all reconstruction.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebEditedResendPrepare {
    expected_host_id: String,
    client_id: String,
    input: nola_domain::edited_resend::PrepareEditedResend,
}
fn validate_edited_resend_web_owner(host: &str, client: &str) -> Result<(), String> {
    if host != host_identity() {
        return Err("edited_resend_host_changed".into());
    }
    if client.trim().is_empty()
        || client.trim() != client
        || client.len() > 512
        || client.chars().any(char::is_control)
    {
        return Err("edited_resend_client_identity_required".into());
    }
    Ok(())
}
fn edited_resend_public_response(
    client: &str,
    prepared: nola_domain::edited_resend::PreparedEditedResend,
) -> Result<Value, String> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let target: Option<Value> =
        app.execute(nola_application::ApplicationCommand::LoadConversation {
            conversation_id: prepared.input.target_conversation_id.clone(),
        })?;
    let target = target.ok_or("edited_resend_target_missing")?;
    Ok(
        json!({"host_id":host_identity(),"client_id":client,"prepared":prepared,
        "target":{"id":target["id"],"title":target["title"],"revision":target["revision"],"configuration_revision":target["configuration_revision"]}}),
    )
}
fn edited_resend_prepare_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let input: WebEditedResendPrepare = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode(422),
                json!({"code":"invalid_edited_resend_intent"}),
            )
        }
    };
    let result = (|| -> Result<Value, String> {
        validate_edited_resend_web_owner(&input.expected_host_id, &input.client_id)?;
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let prepared: nola_domain::edited_resend::PreparedEditedResend =
            app.execute(nola_application::ApplicationCommand::PrepareEditedResend {
                input: input.input,
            })?;
        edited_resend_public_response(&input.client_id, prepared)
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(
            StatusCode(409),
            json!({"code":"edited_resend_rejected","error":error}),
        ),
    }
}
fn edited_resend_load_response(url: &str, target: &str) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Option<Value>, String> {
        let host =
            query_param(url, "expected_host_id").ok_or("edited_resend_host_identity_required")?;
        let client =
            query_param(url, "client_id").ok_or("edited_resend_client_identity_required")?;
        validate_edited_resend_web_owner(&host, &client)?;
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let prepared: Option<nola_domain::edited_resend::PreparedEditedResend> = app.execute(
            nola_application::ApplicationCommand::LoadPreparedEditedResend {
                conversation_id: target.into(),
            },
        )?;
        prepared
            .map(|value| edited_resend_public_response(&client, value))
            .transpose()
    })();
    match result {
        Ok(Some(value)) => json_response(StatusCode(200), value),
        Ok(None) => json_response(
            StatusCode(404),
            json!({"code":"edited_resend_preparation_not_found"}),
        ),
        Err(error) => json_response(
            StatusCode(409),
            json!({"code":"edited_resend_load_rejected","error":error}),
        ),
    }
}
fn web_prepared_edited_resend(
    body: &Value,
) -> Result<Option<nola_domain::edited_resend::PreparedEditedResend>, String> {
    let Some(marker) = body.get("edited_resend_operation_id") else {
        return Ok(None);
    };
    let _ = marker
        .as_str()
        .ok_or("edited_resend_operation_identity_required")?;
    let target = body["conversationId"]
        .as_str()
        .ok_or("edited_resend_target_required")?;
    let host = body["expected_host_id"]
        .as_str()
        .ok_or("edited_resend_host_identity_required")?;
    let client = body["client_id"]
        .as_str()
        .ok_or("edited_resend_client_identity_required")?;
    validate_edited_resend_web_owner(host, client)?;
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let prepared: Option<nola_domain::edited_resend::PreparedEditedResend> = app.execute(
        nola_application::ApplicationCommand::LoadPreparedEditedResend {
            conversation_id: target.into(),
        },
    )?;
    let prepared = prepared.ok_or("edited_resend_preparation_not_found")?;
    validate_web_edited_resend_shape(body, &prepared)?;
    Ok(Some(prepared))
}

fn validate_web_edited_resend_shape(
    body: &Value,
    prepared: &nola_domain::edited_resend::PreparedEditedResend,
) -> Result<(), String> {
    let fields=body.as_object().ok_or("edited_resend_request_object_required")?;
    if fields.keys().any(|key| !matches!(key.as_str(),
        "edited_resend_operation_id"|"conversationId"|"client_id"|"expected_host_id"|
        "submission_id"|"user_message_id"|"assistant_message_id"|"messages"|
        "current_user_display_text"|"timezone"|"language"|"execution_backend"|
        "expected_configuration_revision"|"expected_parent_message_id"|"tool_plan_ref"|"request_digest"|
        "context_components"|"context_messages"|"history"|"permission_profile"|"model_test_mode")) {
        return Err("edited_resend_renderer_configuration_is_forbidden".into());
    }
    if body.get("expected_parent_message_id").is_some_and(|value|!value.is_null())
        || body.get("current_user_display_text").is_some_and(|value|value.as_str()!=Some(prepared.input.edited_text.as_str())) {
        return Err("edited_resend_only_the_new_edited_user_message_is_allowed".into());
    }
    let marker = body["edited_resend_operation_id"]
        .as_str()
        .ok_or("edited_resend_operation_identity_required")?;
    if body["conversationId"].as_str() != Some(prepared.input.target_conversation_id.as_str())
        || prepared.input.operation_id != marker
        || body["submission_id"].as_str() != Some(prepared.run_id.as_str())
        || body["user_message_id"].as_str() != Some(prepared.user_message_id.as_str())
        || body["assistant_message_id"].as_str() != Some(prepared.assistant_message_id.as_str())
    {
        return Err("edited_resend_submission_owner_mismatch".into());
    }
    for field in [
        "model_intent",
        "modelProfileId",
        "model",
        "provider",
        "api_key",
        "endpoint",
        "device_bridge",
        "queued_user_command_id",
        "context_files",
        "host_attachments",
        "attachments",
        "external_session_id",
        "agent_ids",
        "selectedAgentIds",
    ] {
        if body.get(field).is_some() {
            return Err("edited_resend_renderer_configuration_is_forbidden".into());
        }
    }
    for field in ["context_components", "context_messages", "history"] {
        if body
            .get(field)
            .is_some_and(|value| !value.as_array().is_some_and(|items| items.is_empty()))
        {
            return Err("edited_resend_cannot_copy_context_or_history".into());
        }
    }
    if body.get("model_test_mode").and_then(Value::as_bool) == Some(true)
        || body
            .get("permission_profile")
            .is_some_and(|value| value.as_str() != Some("cautious"))
    {
        return Err("edited_resend_requires_fresh_cautious_execution".into());
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("edited_resend_requires_one_edited_user_message")?;
    if messages.len() != 1
        || messages[0]["role"] != "user"
        || messages[0]["content"].as_str() != Some(prepared.input.edited_text.as_str())
        || messages[0].as_object().is_none_or(|fields| {
            fields
                .keys()
                .any(|key| !matches!(key.as_str(), "role" | "content"))
        })
    {
        return Err("edited_resend_requires_only_the_exact_edited_user_message".into());
    }
    Ok(())
}

#[cfg(test)]
mod edited_resend_renderer_contract_tests {
    use super::*;
    fn prepared() -> nola_domain::edited_resend::PreparedEditedResend {
        use nola_domain::edited_resend::*;
        PreparedEditedResend {
            input: PrepareEditedResend {
                operation_id: "operation".into(),
                source_conversation_id: "source".into(),
                source_message_id: "old-user".into(),
                expected_source_revision: 1,
                expected_source_message_revision: 1,
                edited_text: "Edited message".into(),
                target_conversation_id: "target".into(),
                title: "Branch".into(),
                created_at_ms: 1,
            },
            source: MessageSendWitness {
                conversation_id: "source".into(),
                message_id: "old-user".into(),
                run_id: "old-run".into(),
                assistant_message_id: "old-assistant".into(),
                body_sha256: "a".repeat(64),
                invocation_config_sha256: "b".repeat(64),
                model: FrozenResendModel {
                    version: 1,
                    primary: Value::Null,
                    fallbacks: json!([]),
                    generation: Value::Null,
                    output_admission: Value::Null,
                    model_capability: json!({}),
                },
            },
            user_message_id: "new-user".into(),
            assistant_message_id: "new-assistant".into(),
            run_id: "new-run".into(),
            admitted_run_id: None,
        }
    }
    fn body() -> Value {
        json!({"conversationId":"target","edited_resend_operation_id":"operation","submission_id":"new-run","user_message_id":"new-user","assistant_message_id":"new-assistant","messages":[{"role":"user","content":"Edited message"}]})
    }
    #[test]
    fn renderer_cannot_replace_durable_model_or_import_context() {
        let prepared = prepared();
        assert!(validate_web_edited_resend_shape(&body(), &prepared).is_ok());
        for (key, value) in [
            ("model_intent", json!({})),
            ("api_key", json!("secret")),
            ("attachments", json!([])),
            (
                "history",
                json!([{"role":"assistant","content":"old answer"}]),
            ),
            ("permission_profile", json!("allow_all")),
        ] {
            let mut request = body();
            request[key] = value;
            assert!(
                validate_web_edited_resend_shape(&request, &prepared).is_err(),
                "{key}"
            );
        }
        let mut changed = body();
        changed["messages"][0]["content"] = json!("Other message");
        assert!(validate_web_edited_resend_shape(&changed, &prepared).is_err());
        let mut changed = body();
        changed["conversationId"] = json!("source");
        assert!(validate_web_edited_resend_shape(&changed, &prepared).is_err());
    }
    #[test]
    fn ordinary_stream_does_not_require_a_resend_preparation() {
        assert!(web_prepared_edited_resend(
            &json!({"messages":[{"role":"user","content":"hello"}]})
        )
        .unwrap()
        .is_none());
    }
    #[test]
    fn preparation_intent_rejects_host_configuration_fields() {
        let input = serde_json::to_value(&prepared().input).unwrap();
        let mut request = json!({"expected_host_id":"host","client_id":"client","input":input});
        assert!(serde_json::from_value::<WebEditedResendPrepare>(request.clone()).is_ok());
        request["core_config"] = json!({});
        assert!(serde_json::from_value::<WebEditedResendPrepare>(request).is_err());
    }
}

type ExpertHostFailure = nola_application::host_expert_template_operation::HostExpertOperationError;
use nola_application::host_expert_template_operation::HostExpertTemplateOperation;

pub(crate) fn expert_host_authority() -> nola_application::expert_template_capture::HostExpertTemplateAuthority {
    nola_application::expert_template_capture::HostExpertTemplateAuthority {
        actor: nola_core::agent::resource_access::AgentResourcePrincipal {
            kind: nola_core::agent::resource_access::AgentResourcePrincipalKind::LocalUser,
            id: crate::resource_access::LOCAL_WEB_USER_ID.into(),
        }, request_host_id: Some(host_identity().to_owned()),
    }
}

fn expert_capture_context(
    app: &mut crate::application_port::CliApplicationPort,
    input: &nola_domain::expert_message::AdmitExpertConversationMessage,
) -> Result<nola_application::expert_template_capture::HostExpertTemplateCaptureContext, ExpertHostFailure> {
    let authority = expert_host_authority();
    let preview: nola_application::expert_template_capture::ExpertTemplateCapturePreview = app.execute_expert_host_operation(
        &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::PreviewCapture { input: input.clone(), authority: authority.clone() })?;
    expert_capture_from_preview(preview, authority)
}

fn expert_capture_from_preview(
    preview: nola_application::expert_template_capture::ExpertTemplateCapturePreview,
    authority: nola_application::expert_template_capture::HostExpertTemplateAuthority,
) -> Result<nola_application::expert_template_capture::HostExpertTemplateCaptureContext, ExpertHostFailure> {
    use nola_application::expert_template_capture::{HostExpertTemplateCaptureContext, HostExpertTemplateSelection, HostExpertTemplateSelectionSource};
    let host_settings = NolaStore::open_default().and_then(|store| load_agent_settings_from_store(&store))?;
    let mut selections = Vec::with_capacity(preview.participants.len());
    for participant in preview.participants {
        let configuration = participant.defaults.conversation_configuration();
        let selected = match configuration.get("selectedPromptTemplateID") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_str().ok_or("invalid_expert_template_selection")?.to_owned()),
        };
        let mut settings = host_settings.clone();
        if let Some(custom) = participant.client_model_presentation.get("customSystemPrompt").and_then(Value::as_str)
            .or_else(|| participant.model_declaration.parameters.get("customSystemPrompt").and_then(Value::as_str)) {
            settings.custom_system_prompt = custom.to_owned();
        }
        let style = configuration.get("promptResponseStyle").and_then(Value::as_str)
            .or_else(|| participant.client_model_presentation.get("responseStyle").and_then(Value::as_str));
        let body = json!({"responseStyle":style});
        use sha2::Digest;
        let host_user_instructions = web_user_instruction_values(&body, &settings).into_iter().map(|(id,content)| {
            nola_domain::expert_message::ExpertPromptDefinition {
                component_id:id.into(), content_sha256:format!("{:x}",sha2::Sha256::digest(content.as_bytes())),
                content, definition_revision:None, platform:json!("web"),
            }
        }).collect();
        selections.push(HostExpertTemplateSelection {
            context_conversation_id: participant.participant.context_conversation_id,
            context_configuration_revision: participant.context_configuration_revision,
            model_profile: nola_host_protocol::HostModelProfileReference {
                profile_id: participant.model_declaration.profile_id, admitted_revision: participant.model_declaration.profile_revision,
            },
            source: if selected.is_some() { HostExpertTemplateSelectionSource::Explicit } else { HostExpertTemplateSelectionSource::NoTemplate },
            template_id: selected, host_user_instructions,
        });
    }
    Ok(HostExpertTemplateCaptureContext { actor: authority.actor, request_host_id: authority.request_host_id,
        selections, builtins: Vec::new() })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebExpertMessageAdmit {
    expected_host_id: String,
    client_id: String,
    input: nola_domain::expert_message::AdmitExpertConversationMessage,
}
fn expert_admit_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let input: WebExpertMessageAdmit = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode(422), json!({"code":"invalid_expert_message_intent"})),
    };
    expert_http_result(execute_expert_admit(input))
}

fn execute_expert_admit(input: WebExpertMessageAdmit) -> Result<Value, ExpertHostFailure> {
    validate_edited_resend_web_owner(&input.expected_host_id, &input.client_id)?;
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let original: Option<nola_domain::expert_message::ExpertMessageManifest> = app.execute_expert_host_operation(
        &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::LoadManifest {
            conversation_id: input.input.conversation_id.clone(), message_id: input.input.user_message_id.clone(), authority: expert_host_authority(),
        })?;
    let manifest = if let Some(original) = original {
        if original.input != input.input { return Err("expert_message_submission_conflict".into()); }
        original
    } else {
        let context = expert_capture_context(&mut app, &input.input)?;
        app.execute_expert_host_operation(&format!("expert-admit:{}",input.input.submission_id),
            HostExpertTemplateOperation::Admit { input: input.input, context }).map_err(|error|match error {
                ExpertHostFailure::Rejected{message,..}=>ExpertHostFailure::Rejected{code:"expert_admission_rejected".into(),message},
                pending=>pending,
            })?
    };
    Ok(json!({"host_id":host_identity(),"client_id":input.client_id,"manifest":manifest}))
}
fn expert_resend_public_response(client:&str,prepared:nola_domain::expert_message::PreparedExpertResend)
    -> Result<Value,String> {
    let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
    let target:Option<Value>=app.execute(nola_application::ApplicationCommand::LoadConversation {
        conversation_id:prepared.input.target_conversation_id.clone()})?;
    let target=target.ok_or("edited_expert_resend_target_missing")?;
    Ok(json!({"host_id":host_identity(),"client_id":client,"prepared":prepared,
        "target":{"id":target["id"],"title":target["title"],"revision":target["revision"],"configuration_revision":target["configuration_revision"]}}))
}
fn expert_resend_prepare_response(request:&mut Request)->Response<Box<dyn Read+Send>> {
    let body=match read_json_value(request) {Ok(value)=>value,Err(response)=>return response};
    let input:WebEditedResendPrepare=match serde_json::from_value(body) {
        Ok(value)=>value,Err(_)=>return json_response(StatusCode(422),json!({"code":"invalid_edited_expert_resend_intent"})),
    };
    let result=(||->Result<Value,ExpertHostFailure>{
        validate_edited_resend_web_owner(&input.expected_host_id,&input.client_id)?;
        let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
        let prepared:nola_domain::expert_message::PreparedExpertResend=app.execute_expert_host_operation(
            &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::PrepareEditedResend { input:input.input, authority:expert_host_authority() })?;
        expert_resend_public_response(&input.client_id,prepared).map_err(ExpertHostFailure::from)
    })();
    expert_http_result(result)
}
fn expert_resend_load_response(url:&str,target:&str)->Response<Box<dyn Read+Send>> {
    let result=(||->Result<Value,ExpertHostFailure>{
        let (host,client)=expert_web_query_owner(url)?;
        validate_edited_resend_web_owner(&host,&client)?;
        let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
        let prepared:Option<nola_domain::expert_message::PreparedExpertResend>=app.execute_expert_host_operation(
            &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::LoadPreparedResend { conversation_id:target.into(), authority:expert_host_authority() })?;
        expert_resend_public_response(&client,prepared.ok_or("expert_message_not_found")?).map_err(ExpertHostFailure::from)
    })();
    expert_http_result(result)
}
fn expert_source_response(url:&str,conversation:&str,message:&str)->Response<Box<dyn Read+Send>> {
    let result=(||->Result<Value,ExpertHostFailure>{
        let (host,client)=expert_web_query_owner(url)?;
        validate_edited_resend_web_owner(&host,&client)?;
        let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
        let manifest:Option<nola_domain::expert_message::ExpertMessageManifest>=app.execute_expert_host_operation(
            &uuid::Uuid::new_v4().to_string(), HostExpertTemplateOperation::LoadManifest {
                conversation_id:conversation.into(),message_id:percent_decode(message),authority:expert_host_authority()})?;
        Ok(json!({"host_id":host_identity(),"client_id":client,"manifest":manifest}))
    })();
    expert_http_result(result)
}
fn expert_web_query_owner(url:&str)->Result<(String,String),String> {
    Ok((query_param(url,"expected_host_id").ok_or("expert_message_host_identity_required")?,
        query_param(url,"client_id").ok_or("expert_message_client_identity_required")?))
}
fn expert_http_result(result:Result<Value,ExpertHostFailure>)->Response<Box<dyn Read+Send>> {
    match result {
        Ok(value)=>json_response(StatusCode(200),value),
        Err(ExpertHostFailure::OutcomePending { request_id, message, .. }) => json_response(StatusCode(503),
            json!({"code":"outcome_pending","outcome_pending":true,"request_id":request_id,"error":message})),
        Err(ExpertHostFailure::Rejected { code, message }) => json_response(StatusCode(if message=="expert_message_not_found" {404}else{409}),
            json!({"code":code,"error":message})),
    }
}

#[cfg(test)]
mod expert_http_closed_intent_tests {
    use super::*;
    #[test]
    fn renderer_admits_only_one_complete_typed_selection_and_no_host_material() {
        let body=json!({"expected_host_id":"host","client_id":"client","input":{
            "conversation_id":"parent","expected_revision":1,
            "room_id":"0e0a7be2-4b1b-4ec9-99b0-4307249b3333","expected_room_revision":1,
            "submission_id":"90ae506f-8cdd-401b-8bb3-8f035ba11111","user_message_id":"user",
            "text":"Question","created_at_ms":1,"selected_participants":[{
                "participant_id":"participant","agent_id":"expert","agent_revision":1,
                "context_conversation_id":"child"}]}});
        let parsed:WebExpertMessageAdmit=serde_json::from_value(body.clone()).unwrap();
        parsed.input.validate().unwrap();
        for field in ["core_config","model_credential_reference","fresh_resend_policy","old_history"] {
            let mut supplied=body.clone();supplied[field]=json!({});
            assert!(serde_json::from_value::<WebExpertMessageAdmit>(supplied).is_err());
        }
        let mut duplicate=parsed.input.clone();
        duplicate.selected_participants.push(duplicate.selected_participants[0].clone());
        assert!(duplicate.validate().is_err());
        let mut empty=parsed.input;empty.selected_participants.clear();
        assert!(empty.validate().is_err());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebInPlaceResendPrepare {
    expected_host_id: String,
    client_id: String,
    input: nola_domain::edited_resend::PrepareInPlaceResend,
}
fn in_place_resend_prepare_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let input: WebInPlaceResendPrepare = serde_json::from_value(body).map_err(|e| e.to_string())?;
        validate_edited_resend_web_owner(&input.expected_host_id, &input.client_id)?;
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let prepared: nola_domain::edited_resend::PreparedInPlaceResend = app.execute(
            nola_application::ApplicationCommand::PrepareInPlaceResend { input: input.input })?;
        let target: Option<Value> = app.execute(nola_application::ApplicationCommand::LoadConversation {
            conversation_id: prepared.input.conversation_id.clone() })?;
        let target = target.ok_or("in_place_resend_conversation_missing")?;
        Ok(json!({"host_id":host_identity(),"client_id":input.client_id,"prepared":prepared,
            "target":{"id":target["id"],"title":target["title"],"revision":target["revision"],"configuration_revision":target["configuration_revision"]}}))
    })();
    match result { Ok(value)=>json_response(StatusCode(200),value),
        Err(error)=>json_response(StatusCode(409),json!({"code":"in_place_resend_rejected","error":error})) }
}
fn validate_web_in_place_resend(body: &Value) -> Result<(), String> {
    let Some(marker) = body.get("in_place_resend_operation_id") else { return Ok(()) };
    let operation_id = marker.as_str().ok_or("invalid_resend_operation")?;
    validate_edited_resend_web_owner(body["expected_host_id"].as_str().unwrap_or(""),body["client_id"].as_str().unwrap_or(""))?;
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let prepared: Option<nola_domain::edited_resend::PreparedInPlaceResend> = app.execute(
        nola_application::ApplicationCommand::LoadPreparedInPlaceResend { operation_id:operation_id.into() })?;
    let p = prepared.ok_or("in_place_resend_not_prepared")?;
    validate_web_in_place_resend_shape(body, &p)
}
fn validate_web_in_place_resend_shape(body: &Value, p: &nola_domain::edited_resend::PreparedInPlaceResend) -> Result<(), String> {
    if body.as_object().ok_or("invalid_resend_body")?.keys().any(|key| !matches!(key.as_str(),
        "in_place_resend_operation_id"|"conversationId"|"client_id"|"expected_host_id"|"submission_id"|
        "user_message_id"|"assistant_message_id"|"messages"|"current_user_display_text"|"timezone"|"language"|
        "execution_backend"|"expected_configuration_revision"|"expected_parent_message_id"|"tool_plan_ref"|"request_digest"|
        "context_components"|"context_messages"|"history"|"permission_profile"|"model_test_mode")) {
        return Err("resend_renderer_configuration_forbidden".into());
    }
    if body["in_place_resend_operation_id"]!=p.input.operation_id||!p.experts.is_empty() || body["conversationId"]!=p.input.conversation_id || body["submission_id"]!=p.run_id
        ||body["user_message_id"]!=p.user_message_id||body["assistant_message_id"]!=format!("{}:assistant",p.run_id)
        ||body["current_user_display_text"]!=p.input.edited_text
        ||body["messages"]!=json!([{"role":"user","content":p.input.edited_text}]) {
        return Err("in_place_resend_request_changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod in_place_resend_web_tests {
    use super::*;
    #[test]
    fn in_place_resend_keeps_owner_and_binds_exact_prepared_text_without_renderer_credentials() {
        let operation = uuid::Uuid::new_v4().to_string();
        let p = nola_domain::edited_resend::PreparedInPlaceResend {
            input: nola_domain::edited_resend::PrepareInPlaceResend { operation_id: operation.clone(), conversation_id:"same".into(),
                message_id:"original".into(), expected_revision:3, expected_message_revision:2, edited_text:"new text".into(), created_at_ms:1 },
            run_id:operation.clone(), user_message_id:format!("{operation}:user"), experts:vec![],
        };
        let body=json!({"in_place_resend_operation_id":operation,"conversationId":"same","submission_id":operation,
            "user_message_id":p.user_message_id,"assistant_message_id":format!("{operation}:assistant"),
            "current_user_display_text":"new text","messages":[{"role":"user","content":"new text"}],"expected_parent_message_id":"retained-prefix"});
        validate_web_in_place_resend_shape(&body,&p).unwrap();
        for (key,value) in [("conversationId",json!("new-branch")),("user_message_id",json!("wrong")),
            ("api_key",json!("forbidden")),("messages",json!([{"role":"user","content":"changed"}]))] {
            let mut changed=body.clone(); changed[key]=value;
            assert!(validate_web_in_place_resend_shape(&changed,&p).is_err(),"{key}");
        }
    }
}
