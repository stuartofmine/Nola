// SPDX-License-Identifier: Apache-2.0
// Paired read transport for the existing Core Goal and conversation plan.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedProgressRequest {
    expected_host_id: String,
    conversation_id: String,
}

fn paired_conversation_progress_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let input: PairedProgressRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| read_paired_conversation_progress(&mut application, host_identity(), input));
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409), json!({"code":"conversation_progress_unavailable","error":error}), &token),
    }
}

fn read_paired_conversation_progress(
    application: &mut crate::application_port::CliApplicationPort,
    source_host_id: &str,
    input: PairedProgressRequest,
) -> Result<Value, String> {
    use nola_application::ApplicationCommand as C;
    if input.expected_host_id != source_host_id || source_host_id.trim().is_empty()
        || input.conversation_id.trim().is_empty() || input.conversation_id.len() > 512 {
        return Err("progress source or conversation identity mismatch".into());
    }
    // Authenticate the conversation's existence through Core, including an
    // empty conversation which does not yet have a Run/checkpoint or Goal.
    let metadata: Value = application.execute(C::LoadConversationMetadata { conversation_id: input.conversation_id.clone() })?;
    if metadata.is_null() { return Err("conversation not found".into()); }
    let goal: Option<nola_core::agent::GoalProjection> = application.execute(
        C::LoadSessionGoalProjection { conversation_id: input.conversation_id.clone() })?;
    let plan: Value = application.execute(C::ConversationPlan {
        command: nola_application::ConversationPlanCommand::Get { conversation_id: input.conversation_id.clone() },
    })?;
    // These are independent Core projections, not one invented progress total.
    Ok(json!({"source_host_id":source_host_id,"conversation_id":input.conversation_id,
        "goal_projection":goal_projection_response(goal.as_ref()),"plan":plan}))
}

#[cfg(test)]
mod paired_progress_tests {
    use super::*;
    #[test]
    fn paired_progress_uses_core_and_rejects_wrong_source_or_missing_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let mut application = crate::application_port::CliApplicationPort::persistent(&directory.path().join("progress.sqlite")).unwrap();
        let _: Value = application.execute(nola_application::ApplicationCommand::CreateConversation {
            input: nola_application::CreateConversation { id:"progress".into(), title:"Progress".into(),
                settings_json:None, created_at_ms:1, updated_at_ms:1 },
        }).unwrap();
        let read = |host: &str, conversation: &str| PairedProgressRequest {
            expected_host_id:host.into(), conversation_id:conversation.into(),
        };
        let value = read_paired_conversation_progress(&mut application, "source", read("source", "progress")).unwrap();
        assert_eq!(value["source_host_id"], "source");
        assert_eq!(value["conversation_id"], "progress");
        assert!(value["goal_projection"].is_null());
        assert!(value["plan"].is_null());
        assert!(read_paired_conversation_progress(&mut application, "source", read("foreign", "progress")).is_err());
        assert!(read_paired_conversation_progress(&mut application, "source", read("source", "missing")).is_err());
        assert!(serde_json::from_value::<PairedProgressRequest>(json!({"expected_host_id":"source",
            "conversation_id":"progress","command":{"type":"delete_conversation"}})).is_err());
    }
}

// A paired client sends user intent, never a raw Application command or a
// replacement Goal projection. Host time and current execution config stay here.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedGoalControlRequest {
    expected_host_id: String,
    conversation_id: String,
    control: GoalControlInput,
}

fn paired_conversation_goal_response(request: &mut Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let input: PairedGoalControlRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(error) => return mobile_encrypted_json_response(StatusCode(422), json!({"error":error.to_string()}), &token),
    };
    let (status, value) = dispatch_paired_goal_control(host_identity(), input, config);
    mobile_encrypted_json_response(status, value, &token)
}

fn dispatch_paired_goal_control(source_host_id: &str, input: PairedGoalControlRequest, config: &Config) -> (StatusCode, Value) {
    if input.expected_host_id != source_host_id || source_host_id.trim().is_empty()
        || input.control.expected_host_id.as_deref().is_some_and(|host| host != source_host_id)
        || input.conversation_id.trim().is_empty() || input.conversation_id.len() > 512 {
        return (StatusCode(409), json!({"code":"goal_control_source_mismatch","error":"Goal source or conversation identity mismatch"}));
    }
    let (status, mut value) = dispatch_conversation_goal_control(&input.conversation_id, input.control, config);
    value.as_object_mut().expect("Goal control response is an object")
        .insert("source_host_id".into(), json!(source_host_id));
    (status, value)
}

#[cfg(test)]
mod paired_goal_control_tests {
    use super::*;

    #[test]
    fn paired_goal_controls_share_guarded_core_transitions_and_source_checks() {
        let _lock = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Environment {
            fn drop(&mut self) {
                let _ = crate::core_agent_runtime::invalidate_core_agent_session("paired-goal");
                for (key, value) in &self.0 {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let _environment = Environment(["NOLA_HOME", "NOLA_DB_PATH"].into_iter()
            .map(|key| (key, std::env::var_os(key))).collect());
        std::env::set_var("NOLA_HOME", directory.path());
        std::env::set_var("NOLA_DB_PATH", directory.path().join("goal.sqlite"));
        let mut config = Config::from_args([
            "nola".into(), "--project".into(), directory.path().to_string_lossy().into_owned(),
            "--endpoint".into(), "http://127.0.0.1:1/v1".into(), "--model".into(), "fixture".into(),
            "--api-key".into(), "fixture".into(),
        ]).unwrap();
        config.skill_dirs = Some(Vec::new());
        let store = NolaStore::open_default().unwrap();
        store.upsert_model_profile(nola_application::ModelProfile {
            id: "paired-goal-model".into(), display_name: "Fixture".into(), model_name: "fixture".into(),
            provider: "openai-compatible".into(), runtime: "remote".into(),
            source_json: json!({"baseURL":"http://127.0.0.1:1/v1","endpointID":"fixture","wireProtocol":"chat_completions",
                "endpointEnabled":true,"modelEnabled":true,"capabilities":["chat"],
                "executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string(),
            parameters_json: json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":4096,"maxContextLength":32768}).to_string(),
            selected:true, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        }).unwrap();
        store.create_web_conversation(&json!(WebConversation::new(
            "paired-goal".into(), "Goal".into(), now_string(), now_string(),
        ))).unwrap();
        let input = |host: &str, conversation: &str, mut control: Value| {
            control["operationId"] = json!(uuid::Uuid::new_v4().to_string());
            PairedGoalControlRequest {
            expected_host_id: host.into(), conversation_id: conversation.into(),
            control: serde_json::from_value(control).unwrap(),
        }};
        let (status, _) = dispatch_paired_goal_control("host", input("foreign", "paired-goal",
            json!({"action":"start","objective":"Verify paired Goal"})), &config);
        assert_eq!(status, StatusCode(409));
        assert!(crate::session::load_session_goal_projection("paired-goal").unwrap().is_none());
        let (status, _) = dispatch_paired_goal_control("host", input("host", "missing",
            json!({"action":"start","objective":"Verify paired Goal"})), &config);
        assert_eq!(status, StatusCode(404));
        let (status, started) = dispatch_paired_goal_control("host", input("host", "paired-goal",
            json!({"action":"start","objective":"Verify paired Goal"})), &config);
        assert_eq!(status, StatusCode(200), "{started}");
        assert_eq!(started["source_host_id"], "host");
        assert_eq!(started["goalProjection"]["active"]["status"], "running");
        let goal_id = started["goalProjection"]["active"]["id"].clone();
        let initial_token = started["goalProjection"]["control_token"].clone();
        let (status, paused) = dispatch_paired_goal_control("host", input("host", "paired-goal",
            json!({"action":"pause","expectedGoalId":goal_id,"expectedControlToken":initial_token})), &config);
        assert_eq!(status, StatusCode(200), "{paused}");
        assert_eq!(paused["goalProjection"]["active"]["status"], "paused");
        // A stale request cannot resume or cancel a newer control generation.
        let (status, stale) = dispatch_paired_goal_control("host", input("host", "paired-goal",
            json!({"action":"resume","expectedGoalId":goal_id,"expectedControlToken":initial_token})), &config);
        assert_eq!(status, StatusCode(409), "{stale}");
        assert_eq!(stale["goalProjection"]["active"]["status"], "paused");
        assert!(stale["goalProjection"]["applied_operations"][stale["operationId"].as_str().unwrap()]["rejection"].is_string());
        let (status, resumed) = dispatch_paired_goal_control("host", input("host", "paired-goal",
            json!({"action":"resume","expectedGoalId":goal_id,
                "expectedControlToken":paused["goalProjection"]["control_token"]})), &config);
        assert_eq!(status, StatusCode(200), "{resumed}");
        assert_eq!(resumed["goalProjection"]["active"]["status"], "running");
        // Disabling the selected profile blocks future execution, but must
        // never remove the user's ability to cancel the admitted Goal.
        let mut disabled = store.model_profile("paired-goal-model").unwrap().unwrap();
        disabled.enabled = false;
        disabled.revision += 1;
        store.upsert_model_profile(disabled).unwrap();
        let (status, cancelled) = dispatch_paired_goal_control("host", input("host", "paired-goal",
            json!({"action":"cancel","expectedGoalId":goal_id,
                "expectedControlToken":resumed["goalProjection"]["control_token"]})), &config);
        assert_eq!(status, StatusCode(200), "{cancelled}");
        assert_eq!(cancelled["goalProjection"]["active"]["status"], "cancelled");
        // The paired reader sees the same durable authority after reopening.
        let mut reopened = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let read = read_paired_conversation_progress(&mut reopened, "host", PairedProgressRequest {
            expected_host_id:"host".into(), conversation_id:"paired-goal".into(),
        }).unwrap();
        assert_eq!(read["goal_projection"], cancelled["goalProjection"]);
        assert!(reopened.active_run("paired-goal").unwrap().is_none(), "control test must not start model execution");
        assert!(serde_json::from_value::<PairedGoalControlRequest>(json!({"expected_host_id":"host",
            "conversation_id":"paired-goal","control":{"action":"pause"},"command":{"type":"delete_conversation"}})).is_err());
    }
}
