// SPDX-License-Identifier: Apache-2.0
// Presentation transport only. Questions and child ownership come from Core;
// reading or closing this view never answers a question or grants permission.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebUserInputResponse {
    run_id: String,
    response: nola_domain::user_input_wait::RespondToUserInputWait,
}

fn conversation_user_input_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let input: WebUserInputResponse = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(_) => return json_response(StatusCode(422), json!({"code":"invalid_user_input_response"})),
    };
    let result = crate::application_port::CliApplicationPort::persistent_default().and_then(|mut application|
        application.execute::<nola_application::ConversationWaitResponse>(
            nola_application::ApplicationCommand::RespondToConversationWait {
                input: nola_application::RespondToConversationWait {
                    conversation_id: conversation_id.to_owned(), run_id: input.run_id,
                    response: input.response, committed_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            }));
    use nola_application::ConversationWaitResponse;
    match result {
        Ok(ConversationWaitResponse::Applied { wait, resume_policy, .. })
        | Ok(ConversationWaitResponse::Replay { wait, resume_policy }) => {
            // Core already decided Goal and Turn state. Wake the existing work;
            // no synthetic user message, new Run, or separate Resume command.
            let wake_pending = !crate::runtime_effect_supervisor::wake_committed_conversation(&wait.conversation_id).unwrap_or(false);
            wake_web_goal_worker();
            json_response(StatusCode(200), json!({"type":"resolved","wait":wait,
                "resume_policy":resume_policy,"wake_pending":wake_pending}))
        }
        Ok(ConversationWaitResponse::Conflict { wait, reason }) => json_response(StatusCode(409),
            json!({"type":"conflict","wait":wait,"reason":reason})),
        Err(_) => json_response(StatusCode(409), json!({"code":"user_input_response_rejected"})),
    }
}

fn conversation_user_input_waits_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(_) => return json_response(StatusCode(503), json!({"code":"user_input_waits_unavailable"})),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => return json_response(StatusCode(404), json!({"code":"conversation_not_found"})),
        Err(_) => return json_response(StatusCode(503), json!({"code":"user_input_waits_unavailable"})),
    }
    // Reuse the same Application queries as CLI/native presentation. In
    // particular, never relabel a child's wait as a parent-owned question.
    let result = crate::user_input_wait::open(conversation_id).and_then(|waits| {
        crate::user_input_wait::owned(conversation_id, None).map(|task_waits| {
            json!({"conversation_id":conversation_id,"waits":waits,"task_waits":task_waits})
        })
    });
    match result {
        Ok(body) => json_response(StatusCode(200), body),
        Err(_) => json_response(StatusCode(503), json!({"code":"user_input_waits_unavailable"})),
    }
}
