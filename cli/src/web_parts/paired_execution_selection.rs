// SPDX-License-Identifier: Apache-2.0
// Explicit paired conversation selection; never changes a Host/global default.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedExecutionSelectionRequest {
    expected_host_id: String,
    conversation_id: String,
    action: PairedExecutionSelectionAction,
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PairedExecutionSelectionAction {
    Read,
    Select { expected_revision: i64, selection: PairedModelSelection },
    Create { title: String, selection: PairedModelSelection },
}
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PairedModelSelection {
    Inherit,
    Model { profile_id: String, profile_revision: i64 },
}

fn paired_execution_selection_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    paired_execution_selection_response_with(request, || Ok((
        crate::application_port::CliApplicationPort::persistent_default()?, host_identity().to_owned())))
}

fn paired_execution_selection_response_with(request: &mut Request,
    open: impl FnOnce() -> Result<(crate::application_port::CliApplicationPort, String), String>
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) { Ok(value) => value, Err(response) => return response };
    let input = match serde_json::from_value::<PairedExecutionSelectionRequest>(body) {
        Ok(value) => value,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    let result = open().and_then(|(mut app, host)| apply_paired_execution_selection(&mut app, &host, input));
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409),
            json!({"code":"conversation_selection_unconfirmed","error":error}), &token),
    }
}

fn apply_paired_execution_selection(app: &mut crate::application_port::CliApplicationPort,
    host: &str, input: PairedExecutionSelectionRequest) -> Result<Value, String> {
    use nola_application::{ApplicationCommand as C, conversation_configuration::{ConversationExecutionSelection as S, SelectConversationExecutionInput}};
    use nola_domain::conversation_configuration::ConfigurationCommitResult;
    if input.expected_host_id != host || [&input.expected_host_id, &input.conversation_id].iter()
        .any(|id| id.is_empty() || id.len() > 512 || id.trim() != id.as_str() || id.chars().any(char::is_control)) {
        return Err("conversation selection source mismatch".into());
    }
    let metadata: Value = app.execute(C::LoadConversationMetadata { conversation_id: input.conversation_id.clone() })?;
    if !metadata.is_null() {
        let owner: nola_domain::ConversationSourceOwner = app.execute(C::LoadConversationSourceOwner {
            conversation_id: input.conversation_id.clone() })?;
        if !matches!(owner, nola_domain::ConversationSourceOwner::Local) {
            return Err("conversation selection requires this execution source".into());
        }
    }
    let now = Utc::now().timestamp_millis();
    let mut select = |selection: PairedModelSelection, revision: i64| -> Result<SelectConversationExecutionInput, String> {
        let selection = match selection {
            PairedModelSelection::Inherit => S::Inherit,
            PairedModelSelection::Model { profile_id, profile_revision } => {
                let profile: Option<nola_application::ModelProfile> = app.execute(C::Model {
                    command: nola_application::ModelCommand::GetProfile { id: profile_id.clone() } })?;
                let profile = profile.ok_or("selected Host profile is unavailable")?;
                let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "invalid profile source")?;
                if profile.revision != profile_revision || profile_revision < 1 || !profile.enabled
                    || profile.deleted_at_ms.is_some() || profile.runtime != "remote"
                    || source["endpointEnabled"] == false || source["modelEnabled"] == false
                    || !source["capabilities"].as_array().is_some_and(|items| items.contains(&json!("chat"))) {
                    return Err("selected Host profile changed or is unavailable".into());
                }
                S::Model { model_id: format!("configured:{profile_id}"), parameters: Default::default(), presentation: Default::default() }
            }
        };
        Ok(SelectConversationExecutionInput { conversation_id: input.conversation_id.clone(), expected_revision: revision,
            selection, execution_host: Some("local".into()), execution_backend: None, external_execution_sessions: None,
            additional_configuration: None, updated_at_ms: now })
    };
    let result = match input.action {
        PairedExecutionSelectionAction::Read => {
            if metadata.is_null() { json!({"status":"not_found"}) }
            else {
                let settings: Value = serde_json::from_str(metadata["settings_json"].as_str().unwrap_or("{}"))
                    .map_err(|_| "invalid conversation configuration")?;
                json!({"status":"read","configuration_revision":metadata["configuration_revision"],"model_selection_id":settings["modelSelectionID"]})
            }
        }
        PairedExecutionSelectionAction::Select { expected_revision, selection } => {
            let command = select(selection, expected_revision)?;
            let result: ConfigurationCommitResult = app.execute(C::SelectConversationExecution { input: command })?;
            match result {
                ConfigurationCommitResult::Committed { snapshot } => json!({"status":"committed",
                    "configuration_revision":snapshot.configuration_revision,"model_selection_id":snapshot.model_selection_id}),
                other => serde_json::to_value(other).map_err(|error| error.to_string())?,
            }
        }
        PairedExecutionSelectionAction::Create { title, selection } => {
            if title.trim().is_empty() || title.len() > 4096 { return Err("invalid conversation title".into()); }
            // Reuse the canonical selection contract for the initial settings,
            // and let CreateConversation commit creation+configuration atomically.
            let patch = select(selection, 1)?.into_patch()?;
            let settings = patch.patch_json.to_string();
            let selected: Value = serde_json::from_str(&settings).map_err(|error| error.to_string())?;
            let created: Value = app.execute(C::CreateConversation { input: nola_application::CreateConversation {
                id: input.conversation_id.clone(), title, settings_json: Some(settings), created_at_ms: now, updated_at_ms: now } })?;
            if created["created"] == true { json!({"status":"created", "configuration_revision":created["initial_configuration_revision"],
                "model_selection_id":selected["modelSelectionID"]}) }
            else { json!({"status":"already_exists"}) }
        }
    };
    Ok(json!({"source_host_id":host,"conversation_id":input.conversation_id,"result":result}))
}

#[cfg(test)]
mod paired_execution_selection_tests {
    use super::*;
    fn seed(database: &Path) {
        let mut app = crate::application_port::CliApplicationPort::persistent(database).unwrap();
        let _: Value = app.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::UpsertProfile { input: nola_application::ModelProfile {
                id:"chosen".into(), display_name:"Chosen".into(), provider:"openai-compatible".into(), model_name:"test".into(),
                runtime:"remote".into(), source_json:json!({"endpointEnabled":true,"modelEnabled":true,"capabilities":["chat"]}).to_string(),
                parameters_json:"{}".into(), selected:false, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
            }} }).unwrap();
    }
    fn exchange(database: &Path, authorized: bool, payload: Value) -> (u16, Value) {
        let token = format!("selection-test-{}", uuid::Uuid::new_v4());
        mobile_tokens().lock().unwrap().insert(token.clone(), Instant::now());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let database = database.to_owned();
        let worker = thread::spawn(move || {
            let mut request = server.recv_timeout(std::time::Duration::from_secs(10)).unwrap().unwrap();
            let response = paired_execution_selection_response_with(&mut request, ||
                Ok((crate::application_port::CliApplicationPort::persistent(&database)?, "source".into())));
            request.respond(response).unwrap();
        });
        let body = nola_core::mobile_crypto::encrypt_mobile_json(&payload, &token).unwrap();
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(15)).build().unwrap();
        let mut request = client.post(format!("http://{address}/api/mobile/chat/agent/v2/execution-selection")).json(&body);
        if authorized { request = request.bearer_auth(&token); }
        let response = request.send().unwrap(); let status = response.status().as_u16();
        let mut value: Value = response.json().unwrap(); worker.join().unwrap();
        if value["type"] == "encrypted" {
            value = nola_core::mobile_crypto::decrypt_mobile_json(&serde_json::from_value(value).unwrap(), &token).unwrap();
        }
        mobile_tokens().lock().unwrap().remove(&token);
        (status, value)
    }
    #[test]
    fn paired_selection_real_http_creates_atomically_and_cas_preserves_source() {
        let directory = tempfile::tempdir().unwrap(); let database = directory.path().join("selection.sqlite"); seed(&database);
        let request = |action: Value| json!({"expected_host_id":"source","conversation_id":"conversation","action":action});
        let model = json!({"kind":"model","profile_id":"chosen","profile_revision":1});
        let create = request(json!({"type":"create","title":"Explicit selection","selection":model}));
        assert_eq!(exchange(&database, false, create.clone()).0, 401);
        let (status, created) = exchange(&database, true, create.clone()); assert_eq!(status, 200);
        assert_eq!(created["result"]["status"], "created");
        assert_eq!(created["result"]["model_selection_id"], "configured:chosen");
        let revision = created["result"]["configuration_revision"].as_i64().unwrap();
        assert_eq!(exchange(&database, true, create).1["result"]["status"], "already_exists");
        let read = exchange(&database, true, request(json!({"type":"read"}))).1;
        assert_eq!(read["result"]["configuration_revision"], revision);
        let stale = request(json!({"type":"select","expected_revision":revision+10,"selection":{"kind":"inherit"}}));
        assert_eq!(exchange(&database, true, stale).1["result"]["status"], "conflict");
        let wrong = json!({"expected_host_id":"foreign","conversation_id":"conversation","action":{"type":"select","expected_revision":revision,"selection":{"kind":"inherit"}}});
        assert_eq!(exchange(&database, true, wrong).0, 409);
        let unavailable = request(json!({"type":"select","expected_revision":revision,"selection":{"kind":"model","profile_id":"chosen","profile_revision":2}}));
        assert_eq!(exchange(&database, true, unavailable).0, 409);
        assert_eq!(exchange(&database, true, request(json!({"type":"read"}))).1["result"]["model_selection_id"], "configured:chosen");
        let selected = request(json!({"type":"select","expected_revision":revision,"selection":{"kind":"inherit"}}));
        assert_eq!(exchange(&database, true, selected).1["result"]["status"], "committed");
        let mut cold = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let profile: Option<nola_application::ModelProfile> = cold.execute(nola_application::ApplicationCommand::Model {
            command:nola_application::ModelCommand::GetProfile {id:"chosen".into()} }).unwrap();
        assert!(!profile.unwrap().selected, "Per-conversation selection cannot change the global default");
        assert!(serde_json::from_value::<PairedExecutionSelectionRequest>(request(json!({"type":"select","expected_revision":revision,"selection":{"kind":"model","profile_id":"chosen","profile_revision":1},"command":{"type":"delete_conversation"}}))).is_err());
    }
}
