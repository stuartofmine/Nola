// SPDX-License-Identifier: Apache-2.0
// Same-origin conversation route; authority is selected by the local Application.
fn read_ui_json<T: for<'de> serde::Deserialize<'de>>(request: &mut Request) -> Result<T, Response<Box<dyn Read + Send>>> {
    const LIMIT: usize = 128 * 1024;
    let mut body = Vec::new();
    request.as_reader().take((LIMIT + 1) as u64).read_to_end(&mut body)
        .map_err(|error| json_response(StatusCode(400), json!({"error":error.to_string()})))?;
    if body.len() > LIMIT {
        return Err(json_response(StatusCode(413), json!({"error":"UI request exceeds byte budget"})));
    }
    let value = nola_core::strict_json::decode(&body)
        .map_err(|error| json_response(StatusCode(400), json!({"error":error.to_string()})))?;
    serde_json::from_value(value)
        .map_err(|error| json_response(StatusCode(400), json!({"error":error.to_string()})))
}

fn conversation_ui_surfaces_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Vec<nola_domain::generative_ui::UiSnapshot>>(
            nola_application::ApplicationCommand::ListUiSurfaces { conversation_id: conversation_id.into() }));
    match result {
        Ok(surfaces) => json_response(StatusCode(200), json!({"surfaces": surfaces})),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

fn conversation_ui_revision_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let reference = match read_ui_json::<nola_domain::generative_ui::UiSurfaceRevisionRef>(request) {
        Ok(reference) => reference, Err(response) => return response,
    };
    if reference.identity.owner.conversation_id != conversation_id || reference.validate().is_err() {
        return json_response(StatusCode(400), json!({"error":"Invalid UI historical reference"}));
    }
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Option<nola_domain::generative_ui::UiSnapshot>>(
            nola_application::ApplicationCommand::ReadUiSurfaceRevision {
                identity: reference.identity, revision: reference.revision,
            }));
    match result {
        Ok(snapshot) => json_response(StatusCode(200), json!({"snapshot":snapshot})),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UiChangesQuery {
    identity: nola_domain::generative_ui::UiSurfaceIdentity,
    base_revision: Option<u64>,
}

impl UiChangesQuery {
    fn valid(&self) -> bool {
        self.identity.validate().is_ok()
            && self.base_revision.is_none_or(|revision| revision > 0 && revision <= i64::MAX as u64)
    }
    fn execute(self) -> Result<Value, String> {
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            nola_application::ApplicationCommand::ReadUiSurfaceChanges {
                identity: self.identity, base_revision: self.base_revision,
            })
    }
}

#[cfg(test)]
mod ui_changes_request_tests {
    use super::*;

    #[test]
    fn baseline_is_optional_but_never_zero_or_outside_storage_range() {
        let identity = json!({"owner":{
            "authority_id":"host", "conversation_id":"conversation", "participant_id":"agent",
            "context_id":"conversation", "originating_run_id":"run", "message_id":"message", "part_id":"part"
        },"surface_id":"form","generation":1});
        for revision in [Value::Null, json!(1), json!(i64::MAX)] {
            let query: UiChangesQuery = serde_json::from_value(json!({"identity":identity,"base_revision":revision})).unwrap();
            assert!(query.valid());
        }
        for revision in [json!(0), json!(u64::MAX)] {
            let query: UiChangesQuery = serde_json::from_value(json!({"identity":identity,"base_revision":revision})).unwrap();
            assert!(!query.valid());
        }
        assert!(serde_json::from_value::<UiChangesQuery>(json!({
            "identity":identity,"base_revision":null,"alternate_host":"untrusted"
        })).is_err());
    }

    #[test]
    fn draft_preview_requires_source_and_rejects_action_or_oversized_body() {
        let body = json!({
            "expected_host_id":"host",
            "identity":{"owner":{
                "authority_id":"host", "conversation_id":"conversation", "participant_id":"agent",
                "context_id":"conversation", "originating_run_id":"run", "message_id":"message", "part_id":"part"
            },"surface_id":"form","generation":1},
            "expected_surface_revision":2,"edits":[],"draft_generation":3
        });
        let input = decode_paired_ui_preview(body.clone(), "host").unwrap();
        assert_eq!(input.expected_surface_revision, 2);
        assert_eq!(input.draft_generation, 3);
        assert!(decode_paired_ui_preview(body.clone(), "other-host").is_err());
        let mut no_source = body.clone();
        no_source.as_object_mut().unwrap().remove("expected_host_id");
        assert!(decode_paired_ui_preview(no_source, "host").is_err());
        let mut with_action = body.clone();
        with_action["action"] = json!({"name":"submit"});
        assert!(decode_paired_ui_preview(with_action, "host").is_err());
        let mut oversized = body;
        oversized["identity"]["surface_id"] = json!("x".repeat(128 * 1024));
        assert!(decode_paired_ui_preview(oversized, "host").is_err());
    }
}

fn conversation_ui_changes_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let query = match read_ui_json::<UiChangesQuery>(request) {
        Ok(query) => query, Err(response) => return response,
    };
    if !query.valid() || query.identity.owner.conversation_id != conversation_id {
        return json_response(StatusCode(400), json!({"error":"Invalid UI change reference"}));
    }
    match query.execute() {
        Ok(update) => json_response(StatusCode(200), json!({"update":update})),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

// Draft evaluation uses the same authoritative Core projection as submission,
// without committing edits, allocating a revision, or dispatching an action.
fn execute_ui_preview(input: nola_domain::generative_ui::UiDraftPreviewInput)
    -> Result<nola_domain::generative_ui::UiDraftPreview, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        nola_application::ApplicationCommand::PreviewUiSurface { input })
}

fn conversation_ui_preview_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let input = match read_ui_json::<nola_domain::generative_ui::UiDraftPreviewInput>(request) {
        Ok(input) => input, Err(response) => return response,
    };
    if input.identity.owner.conversation_id != conversation_id {
        return json_response(StatusCode(400), json!({"error":"UI preview conversation mismatch"}));
    }
    match execute_ui_preview(input) {
        Ok(preview) => json_response(StatusCode(200), json!({"preview":preview})),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

fn decode_paired_ui_preview(mut body: Value, expected_host: &str)
    -> Result<nola_domain::generative_ui::UiDraftPreviewInput, String> {
    if serde_json::to_vec(&body).map_or(true, |bytes| bytes.len() > 128 * 1024) {
        return Err("UI preview exceeds byte budget".into());
    }
    let host = body.as_object_mut().and_then(|body| body.remove("expected_host_id"));
    if host.as_ref().and_then(Value::as_str) != Some(expected_host) {
        return Err("UI source Host changed".into());
    }
    serde_json::from_value(body).map_err(|error| error.to_string())
}

fn mobile_ui_preview_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let input = match decode_paired_ui_preview(body, &host_identity()) {
        Ok(input) => input,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error}), &token),
    };
    match execute_ui_preview(input) {
        Ok(preview) => mobile_encrypted_json_response(StatusCode(200),
            json!({"source_host_id":host_identity(),"result":preview}), &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409), json!({"error":error}), &token),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedUiChangesQuery {
    expected_host_id: String,
    identity: nola_domain::generative_ui::UiSurfaceIdentity,
    base_revision: Option<u64>,
}

fn mobile_ui_changes_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let paired: PairedUiChangesQuery = match serde_json::from_value(body) {
        Ok(query) => query,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    if paired.expected_host_id != host_identity() {
        return mobile_encrypted_json_response(StatusCode(409), json!({"error":"UI source Host changed"}), &token);
    }
    let query = UiChangesQuery { identity: paired.identity, base_revision: paired.base_revision };
    if !query.valid() {
        return mobile_encrypted_json_response(StatusCode(422), json!({"error":"Invalid UI change reference"}), &token);
    }
    match query.execute() {
        Ok(update) => mobile_encrypted_json_response(StatusCode(200),
            json!({"source_host_id":host_identity(),"result":update}), &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409), json!({"error":error}), &token),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebUiInteractionRequest {
    request_id: String,
    input: nola_application::generative_ui::SubmitUiInteraction,
}

fn conversation_ui_interaction_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let body = match read_ui_json::<WebUiInteractionRequest>(request) {
        Ok(body) => body,
        Err(response) => return response,
    };
    // Scope the receipt to this route and require a bounded opaque browser ID.
    if body.request_id.len() > 128 || body.request_id.is_empty()
        || !body.request_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || body.input.submission_id != body.request_id
        || body.input.interaction.identity.owner.conversation_id != conversation_id
    {
        return json_response(StatusCode(400), json!({"status":"rejected","message":"Invalid UI interaction identity"}));
    }
    let mut application = match crate::application_port::CliApplicationPort::persistent_default() {
        Ok(application) => application,
        Err(error) => return json_response(StatusCode(503), json!({"status":"rejected","message":error})),
    };
    let receipt_id = format!("ui-interaction:{}:{}", conversation_id, body.request_id);
    json_response(StatusCode(200), submit_ui_interaction_result(&mut application, &receipt_id, body.input))
}

fn submit_ui_interaction_result(application: &mut crate::application_port::CliApplicationPort,
    receipt_id: &str, input: nola_application::generative_ui::SubmitUiInteraction) -> Value {
    match application.submit_ui_interaction(receipt_id, input) {
        Ok(receipt) => json!({"status":"committed","receipt":receipt}),
        Err(error) => {
            let status = if error.requires_reconciliation() { "unknown" }
                else if matches!(&error, nola_service_client::ApplicationServiceCommandError::Conflict { .. }) { "conflict" }
                else { "rejected" };
            json!({"status":status,"message":error.to_string()})
        }
    }
}

// Read-only wake-up hint. Claim and new-Run admission still arbitrate races.
fn conversation_queued_input_response(conversation_id: &str, url: &str) -> Response<Box<dyn Read + Send>> {
    let result = (|| {
        let host = query_param(url, "expected_host_id").ok_or("expected_host_id required")?;
        let client = query_param(url, "client_id").ok_or("client_id required")?;
        queued_ui_dispatch(conversation_id, &host, &client)
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

fn queued_ui_dispatch(conversation: &str, host: &str, client: &str) -> Result<Value, String> {
    if host != host_identity() || client.trim().is_empty() || client.chars().any(char::is_control) {
        return Err("queued action source/client mismatch".into());
    }
    let head = idle_ui_queue_head(conversation)?;
    let dispatch: Option<nola_domain::remote_submission::SourceSubmissionReceipt> = if let Some(head) = &head {
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        application.execute(nola_application::ApplicationCommand::ObservedSourceSubmissionReceipt {
            source_host_id:host.into(),client_id:client.into(),conversation_id:conversation.into(),
            run_id:head.command_id.clone(),assistant_message_id:format!("{}:assistant",head.command_id),
        })?
    } else { None };
    Ok(json!({"head":head,"dispatch":dispatch}))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelQueuedUiActionQuery {
    expected_host_id: String, client_id: String, command_id: String, request_digest: String,
}
fn cancel_queued_ui_action(conversation: &str, query: CancelQueuedUiActionQuery) -> Result<Value, String> {
    if query.expected_host_id != host_identity() { return Err("queued action source mismatch".into()); }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let receipt: Option<nola_domain::remote_submission::SourceSubmissionReceipt> = application.execute(
        nola_application::ApplicationCommand::ObservedSourceSubmissionReceipt {
            source_host_id:query.expected_host_id,client_id:query.client_id,conversation_id:conversation.into(),
            run_id:query.command_id.clone(),assistant_message_id:format!("{}:assistant",query.command_id),
        })?;
    let receipt = receipt.ok_or("queued action has no dispatch receipt")?;
    if receipt.request_digest != query.request_digest { return Err("queued action digest mismatch".into()); }
    application.execute(nola_application::ApplicationCommand::CancelRejectedQueuedUiAction {
        identity:receipt.identity,request_digest:receipt.request_digest,now_ms:chrono::Utc::now().timestamp_millis(),
    })
}
fn conversation_cancel_queued_input_response(request: &mut Request, conversation: &str) -> Response<Box<dyn Read + Send>> {
    let query = match read_ui_json(request) { Ok(value) => value, Err(response) => return response };
    match cancel_queued_ui_action(conversation, query) {
        Ok(value) => json_response(StatusCode(200),value),
        Err(error) => json_response(StatusCode(409),json!({"error":error})),
    }
}

fn idle_ui_queue_head(conversation_id: &str) -> Result<Option<nola_application::AgentCommandHead>, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if application.active_run(conversation_id)?.is_some() { return Ok(None); }
    application.execute(nola_application::ApplicationCommand::PeekAgentCommand {
        session_id: conversation_id.into(),
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedUiRevisionQuery {
    expected_host_id: String,
    identity: nola_domain::generative_ui::UiSurfaceIdentity,
    revision: u64,
}

fn mobile_ui_revision_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let query: PairedUiRevisionQuery = match serde_json::from_value(body) {
        Ok(query) => query,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    if query.expected_host_id != host_identity() {
        return mobile_encrypted_json_response(StatusCode(409), json!({"error":"UI source Host changed"}), &token);
    }
    let reference = nola_domain::generative_ui::UiSurfaceRevisionRef { identity: query.identity, revision: query.revision };
    if reference.validate().is_err() {
        return mobile_encrypted_json_response(StatusCode(422), json!({"error":"Invalid UI historical reference"}), &token);
    }
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Option<nola_domain::generative_ui::UiSnapshot>>(
            nola_application::ApplicationCommand::ReadUiSurfaceRevision {
                identity: reference.identity, revision: reference.revision,
            }));
    match result {
        Ok(snapshot) => mobile_encrypted_json_response(StatusCode(200),
            json!({"source_host_id":host_identity(),"result":snapshot}), &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409), json!({"error":error}), &token),
    }
}

fn require_paired_ui_epoch(expected: &str) -> Result<(), String> {
    let actual: String = crate::application_port::CliApplicationPort::persistent_default()?.execute(
        nola_application::ApplicationCommand::ReadUiAuthority)?;
    if expected.trim().is_empty() || expected != actual { return Err("UI source Store epoch changed".into()); }
    Ok(())
}

fn mobile_ui_queued_input_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else { return json_response(StatusCode(401), json!({"error":"unauthorized"})); };
    let body = match read_mobile_json_value(request, &token) { Ok(body) => body, Err(response) => return response };
    let result = (|| {
        let epoch = body.get("expected_source_store_epoch").and_then(Value::as_str).ok_or("expected_source_store_epoch required")?;
        require_paired_ui_epoch(epoch)?;
        let conversation = body.get("conversation_id").and_then(Value::as_str).ok_or("conversation_id required")?;
        let host = body.get("expected_host_id").and_then(Value::as_str).ok_or("expected_host_id required")?;
        let client = body.get("client_id").and_then(Value::as_str).ok_or("client_id required")?;
        queued_ui_dispatch(conversation, host, client)
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200),json!({"source_host_id":host_identity(),"result":value}),&token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409),json!({"error":error}),&token),
    }
}
fn mobile_cancel_queued_input_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else { return json_response(StatusCode(401), json!({"error":"unauthorized"})); };
    let mut body = match read_mobile_json_value(request, &token) { Ok(body) => body, Err(response) => return response };
    let result = (|| {
        let epoch = body.as_object_mut().and_then(|body| body.remove("expected_source_store_epoch"))
            .and_then(|value|value.as_str().map(str::to_owned)).ok_or("expected_source_store_epoch required")?;
        require_paired_ui_epoch(&epoch)?;
        let conversation = body.as_object_mut().and_then(|body| body.remove("conversation_id"))
            .and_then(|value|value.as_str().map(str::to_owned)).ok_or("conversation_id required")?;
        let query = serde_json::from_value(body).map_err(|error|error.to_string())?;
        cancel_queued_ui_action(&conversation,query)
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200),json!({"source_host_id":host_identity(),"result":value}),&token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409),json!({"error":error}),&token),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedUiSurfaceQuery {
    expected_host_id: String,
    expected_source_store_epoch: String,
    conversation_id: String,
}

impl PairedUiSurfaceQuery {
    fn has_valid_source(&self) -> bool {
        self.expected_host_id == host_identity() && !self.conversation_id.is_empty()
            && self.conversation_id.len() <= 512 && self.conversation_id.trim() == self.conversation_id
            && !self.conversation_id.chars().any(char::is_control)
    }
}

fn mobile_ui_surfaces_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let query: PairedUiSurfaceQuery = match serde_json::from_value(body) {
        Ok(query) => query,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    if !query.has_valid_source() || require_paired_ui_epoch(&query.expected_source_store_epoch).is_err() {
        return mobile_encrypted_json_response(StatusCode(409), json!({"error":"UI source or conversation identity mismatch"}), &token);
    }
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Vec<nola_domain::generative_ui::UiSnapshot>>(
            nola_application::ApplicationCommand::ListUiSurfaces { conversation_id: query.conversation_id }));
    match result {
        Ok(surfaces) => mobile_encrypted_json_response(StatusCode(200),
            json!({"source_host_id":host_identity(),"result":surfaces}), &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409), json!({"error":error}), &token),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedUiInteractionRequest {
    expected_host_id: String,
    input: nola_application::generative_ui::SubmitUiInteraction,
}

fn mobile_ui_interaction_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    if serde_json::to_vec(&body).map_or(true, |bytes| bytes.len() > 128 * 1024) {
        return mobile_encrypted_json_response(StatusCode(413), json!({"error":"UI interaction exceeds budget"}), &token);
    }
    let input: PairedUiInteractionRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    if input.expected_host_id != host_identity() {
        return mobile_encrypted_json_response(StatusCode(409), json!({"error":"UI source Host changed"}), &token);
    }
    let mut application = match crate::application_port::CliApplicationPort::persistent_default() {
        Ok(application) => application,
        Err(error) => return mobile_encrypted_json_response(StatusCode(503), json!({"error":error}), &token),
    };
    let receipt_id = format!("ui-interaction:{}:{}",
        input.input.interaction.identity.owner.conversation_id, input.input.submission_id);
    let result = submit_ui_interaction_result(&mut application, &receipt_id, input.input);
    mobile_encrypted_json_response(StatusCode(200),
        json!({"source_host_id":host_identity(),"result":result}), &token)
}

#[cfg(test)]
mod paired_ui_epoch_tests {
    use super::*;
    #[test]
    fn paired_ui_reads_require_the_exact_core_store_epoch() {
        let _lock = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Environment {
            fn drop(&mut self) { for (key,value) in &self.0 { match value {
                Some(value)=>std::env::set_var(key,value), None=>std::env::remove_var(key),
            } } }
        }
        let _environment = Environment(["NOLA_HOME","NOLA_DB_PATH"].into_iter().map(|key|(key,std::env::var_os(key))).collect());
        std::env::set_var("NOLA_HOME",root.path());
        std::env::set_var("NOLA_DB_PATH",root.path().join("epoch.sqlite"));
        let epoch: String = crate::application_port::CliApplicationPort::persistent_default().unwrap()
            .execute(nola_application::ApplicationCommand::ReadUiAuthority).unwrap();
        assert!(require_paired_ui_epoch(&epoch).is_ok());
        assert!(require_paired_ui_epoch("another-store").is_err());
        assert!(require_paired_ui_epoch("").is_err());
        assert!(serde_json::from_value::<PairedUiSurfaceQuery>(json!({
            "expected_host_id":host_identity(),"conversation_id":"conversation"
        })).is_err());
        let query: PairedUiSurfaceQuery = serde_json::from_value(json!({
            "expected_host_id":host_identity(),"expected_source_store_epoch":epoch,"conversation_id":"conversation"
        })).unwrap();
        assert!(query.has_valid_source());
    }
}
