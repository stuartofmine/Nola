// SPDX-License-Identifier: Apache-2.0
//! Shared JSON session projection; neither CLI rendering nor Host reconstruction.
use crate::session;
use serde_json::{json, Value};

pub(crate) fn session_show_json_value(
    saved: &session::SavedSession,
    identified_messages: &[Value],
    trace_runs: &[session::SessionTraceRun],
    conversation_projection: &Value,
    task_projection: Option<&nola_core::agent::task::TaskProjection>,
    memory_runtime_projection: Option<&nola_core::agent::MemoryRuntimeProjection>,
    goal_projection: Option<&nola_core::agent::GoalProjection>,
    permission_projection: Option<&nola_core::agent::permissions::PermissionProjection>,
    skill_projection: Option<&session::SessionSkillProjection>,
) -> Value {
    let context_usage = conversation_projection
        .get("context_usage")
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "id": saved.id,
        "created_at": saved.created_at,
        "updated_at": saved.updated_at,
        "messages": identified_messages,
        "trace_runs": trace_runs,
        "conversation_projection": conversation_projection,
        "task_projection": task_projection,
        "memory_runtime_projection": memory_runtime_projection,
        "goal_projection": goal_projection,
        "permission_projection": permission_projection,
        "skill_runtime_projection": skill_projection,
        "context_usage": context_usage,
    })
}

#[cfg(test)]
mod session_show_projection_contract_tests {
    use super::session_show_json_value;
    use crate::session::{SavedSession, SessionSkillProjection, SessionTraceRun};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn json_contract_exposes_application_projection_without_rebuilding_parts() {
        let projection = json!({
            "contract_version": 1,
            "conversation_id": "session-1",
            "revision": 7,
            "title": "Session 1",
            "messages": [{
                "message_id": "session-1:turn:1:assistant",
                "role": "assistant",
                "state": "completed",
                "revision": 7,
                "created_at_ms": 1,
                "parts": [{
                    "kind": "text",
                    "part_id": "text-1",
                    "revision": 7,
                    "markdown": "Hello",
                    "is_final": true
                }]
            }],
            "active_run": null,
            "context_usage": {
                "used_tokens": 42,
                "context_window_tokens": 65536,
                "prompt_token_limit": 60000,
                "estimated": false,
                "measured_at_revision": 7
            }
        });
        let saved = SavedSession {
            id: "session-1".to_string(),
            created_at: "2026-08-02T00:00:00Z".to_string(),
            updated_at: "2026-08-02T00:01:00Z".to_string(),
            messages: vec![json!({"role": "user", "content": "Hello"})],
        };
        let identified_messages = vec![json!({
            "id": "session-1:00000000",
            "message_id": "session-1:00000000",
            "role": "user",
            "content": "Hello"
        })];
        let value = session_show_json_value(
            &saved,
            &identified_messages,
            &Vec::<SessionTraceRun>::new(),
            &projection,
            None,
            Some(&nola_core::agent::MemoryRuntimeProjection::default()),
            None,
            Some(&nola_core::agent::permissions::PermissionProjection::default()),
            Some(&SessionSkillProjection {
                session_id: "session-1".to_string(),
                cursor: 19,
                revision: 4,
                catalog_revision: 3,
                order: Vec::new(),
                runs: BTreeMap::new(),
            }),
        );

        assert_eq!(value["conversation_projection"], projection);
        assert_eq!(value["conversation_projection"]["revision"], 7);
        assert_eq!(
            value["conversation_projection"]["messages"][0]["message_id"],
            "session-1:turn:1:assistant"
        );
        assert_eq!(
            value["conversation_projection"]["messages"][0]["parts"][0]["markdown"],
            "Hello"
        );
        assert_eq!(value["context_usage"]["used_tokens"], 42);
        assert_eq!(value["context_usage"]["context_window_tokens"], 65536);
        assert_eq!(value["messages"][0]["message_id"], "session-1:00000000");
        assert_eq!(value["messages"][0]["id"], "session-1:00000000");
        assert!(value["messages"][0].get("turn_id").is_none());
        assert_eq!(value["memory_runtime_projection"]["revision"], 0);
        assert_eq!(value["memory_runtime_projection"]["job_order"], json!([]));
        assert_eq!(value["permission_projection"]["revision"], 0);
        assert_eq!(value["permission_projection"]["pending"], json!([]));
        assert_eq!(value["skill_runtime_projection"]["sessionId"], "session-1");
        assert_eq!(value["skill_runtime_projection"]["cursor"], 19);
        assert_eq!(value["skill_runtime_projection"]["revision"], 4);
        assert_eq!(value["skill_runtime_projection"]["catalogRevision"], 3);
        assert!(value["skill_runtime_projection"]
            .get("session_id")
            .is_none());
    }
}
