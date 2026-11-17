// SPDX-License-Identifier: Apache-2.0

use super::*;

fn editable_definition() -> Value {
    json!({
        "name": "Editable",
        "format": "script",
        "script": "printf never-executed",
        "cwd": "/tmp",
        "userPrompt": "Keep this note",
        "frequency": "daily",
        "time": "09:00",
        "timezone": "local",
        "executionSpec": {
            "kind": "agent",
            "spec": {
                "prompt": "Inspect",
                "execution_scope": {"kind": "local_workspace", "root": "/tmp"},
                "permission_profile": "cautious",
                "model_selection": "use_current_default_at_admission",
                "context_refs": [],
                "skill_refs": []
            }
        }
    })
}

#[test]
fn web_automation_admission_lifecycle_and_validation() {
    use sha2::{Digest, Sha256};

    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("automations.sqlite")).unwrap();

    // 1. Script automation creation, edit revision CAS, and approval revocation
    let mut action =
        build_web_scheduled_action(serde_json::from_value(editable_definition()).unwrap()).unwrap();
    assert!(action.user_prompt.contains("Keep this note"));
    action.script_approved = Some(true);
    action.enabled = true;
    let original: WebScheduledAction =
        serde_json::from_value(store.upsert_web_scheduled_action(&json!(action)).unwrap()).unwrap();
    let mut draft = editable_definition();
    draft["time"] = json!("10:30");
    let edited =
        edit_web_scheduled_action(&original, serde_json::from_value(draft).unwrap()).unwrap();
    assert_eq!(edited.id, original.id);
    assert!(!edited.enabled);
    assert_eq!(edited.script_approved, Some(false));
    let saved = store
        .update_web_scheduled_action_if_revision(&json!(edited), original.revision)
        .unwrap()
        .unwrap();
    assert_eq!(saved["time"], "10:30");
    assert!(saved["revision"].as_i64().unwrap() > original.revision);
    assert!(store
        .update_web_scheduled_action_if_revision(&json!(edited), original.revision)
        .unwrap()
        .is_none());

    // 2. Reject smuggled approval fields and invalid time formats
    for key in ["code", "workingDirectory", "approveNow", "approved", "user_prompt"] {
        let mut smuggled = editable_definition();
        smuggled[key] = json!(true);
        assert!(serde_json::from_value::<ScheduledActionInput>(smuggled).is_err(), "{key}");
    }
    let mut bad_time = editable_definition();
    bad_time["time"] = json!("25:00");
    assert!(build_web_scheduled_action(serde_json::from_value(bad_time).unwrap()).is_err());

    // 3. Prompt automation round-trip on the same store without source metadata
    let mut prompt_draft = editable_definition();
    prompt_draft["format"] = json!("agent");
    prompt_draft.as_object_mut().unwrap().remove("script");
    prompt_draft.as_object_mut().unwrap().remove("cwd");
    let prompt_action =
        build_web_scheduled_action(serde_json::from_value(prompt_draft.clone()).unwrap()).unwrap();
    assert!(prompt_action.url.is_empty());
    assert!(prompt_action.enabled);
    assert_eq!(prompt_action.source_kind, "agent");
    assert_eq!(prompt_action.script_approved, None);
    let saved_prompt = store.upsert_web_scheduled_action(&json!(prompt_action)).unwrap();
    let loaded = store.load_web_scheduled_actions().unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.iter().any(|item| item["id"] == saved_prompt["id"]
        && item["executionSpec"] == prompt_draft["executionSpec"]));
    prompt_draft["url"] = json!("https://should-not-be-required.test");
    assert!(build_web_scheduled_action(serde_json::from_value(prompt_draft).unwrap()).is_err());

    // 4. Skill catalog source resolution and content hash freezing
    let source = root.path().join("review");
    std::fs::create_dir(&source).unwrap();
    let content = "---\nname: review\ndescription: isolated review skill\n---\nReview carefully.\n";
    std::fs::write(source.join("SKILL.md"), content).unwrap();
    let runtime =
        crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![source.clone()]));
    let catalog = runtime.index().unwrap();
    let id = catalog[0]["id"].as_str().unwrap().to_string();
    let artifacts = automation_skill_artifacts(&runtime, &[id.clone()]).unwrap();
    assert_eq!(artifacts[0].path, source.join("SKILL.md").canonicalize().unwrap());
    assert_eq!(artifacts[0].sha256, format!("{:x}", Sha256::digest(content.as_bytes())));
    assert!(automation_skill_artifacts(&runtime, &[id.clone(), id.clone()]).is_err());
    assert!(automation_skill_artifacts(&runtime, &["../unselected".into()]).is_err());
    let disabled = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![]));
    assert!(automation_skill_artifacts(&disabled, &[id.clone()]).is_err());
    std::fs::write(source.join("SKILL.md"), content.replace("carefully", "thoroughly")).unwrap();
    let updated = automation_skill_artifacts(&runtime, &[id]).unwrap();
    assert_ne!(artifacts[0].sha256, updated[0].sha256);

    // 5. Execution scope authority & unbound source rejection
    let mut spec_value = json!({
        "kind": "agent",
        "spec": {
            "prompt": "Inspect the requested workspace",
            "execution_scope": {"kind": "local_workspace", "root": "/tmp"},
            "permission_profile": "cautious",
            "model_selection": "use_current_default_at_admission",
            "context_refs": [],
            "skill_refs": []
        }
    });
    assert!(web_automation_spec(&serde_json::from_value(spec_value.clone()).unwrap()).is_ok());
    spec_value["spec"]["context_refs"] = json!([{"kind": "web_source", "id": "unbound-source"}]);
    assert!(web_automation_spec(&serde_json::from_value(spec_value.clone()).unwrap()).is_err());
    spec_value["spec"]["context_refs"] = json!([]);
    spec_value["spec"]["execution_scope"] = json!({"kind": "device_local", "host_id": "another-host"});
    assert!(web_automation_spec(&serde_json::from_value(spec_value).unwrap()).is_err());
    assert!(serde_json::from_value::<ScheduledActionInput>(json!({"name": "missing authority"}))
        .is_err());
}

#[test]
fn goal_wire_contract_projects_core_token_and_rejects_legacy_revision_control() {
    use nola_core::agent::{GoalProjection, GoalRecordStatus, GoalRequirement};

    assert_eq!(goal_projection_response(None), Value::Null);
    let mut projection = GoalProjection::default();
    projection
        .start(
            "goal".into(),
            "verify".into(),
            vec![],
            vec![GoalRequirement {
                acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1,
                id: "requirement".into(),
                description: "verify result".into(),
                dependencies: vec![],
                status: GoalRecordStatus::Pending,
                evidence_refs: vec![],
            }],
            1,
        )
        .unwrap();
    let response = goal_projection_response(Some(&projection));
    assert_eq!(response["control_token"], json!(projection.control_token()));
    assert!(response["control_token"].as_str().is_some_and(|token| !token.is_empty()));
    assert_eq!(response["revision"], json!(projection.revision));
    assert_eq!(response["active"]["id"], "goal");
    assert_eq!(
        (
            response["can_pause"].as_bool(),
            response["can_resume"].as_bool(),
            response["can_cancel"].as_bool()
        ),
        (Some(true), Some(false), Some(true))
    );
    projection.pause(nola_core::agent::goal::GoalStatus::NeedsUser, None, 2).unwrap();
    let waiting = goal_projection_response(Some(&projection));
    assert_eq!(waiting["active"]["status"], "needs_user");
    assert_eq!(waiting["can_pause"], true);
    assert_eq!(waiting["can_resume"], true);
    assert_eq!(waiting["can_cancel"], true);
    let empty = goal_projection_response(Some(&GoalProjection::default()));
    assert_eq!(
        (
            empty["can_pause"].as_bool(),
            empty["can_resume"].as_bool(),
            empty["can_cancel"].as_bool()
        ),
        (Some(false), Some(false), Some(false))
    );

    let parsed: GoalControlInput = serde_json::from_value(json!({
        "action": "pause",
        "operationId": "pause-fixture",
        "expectedGoalId": "goal",
        "expectedControlToken": "core-token"
    }))
    .unwrap();
    assert_eq!(parsed.expected_control_token.as_deref(), Some("core-token"));
    assert!(serde_json::from_value::<GoalControlInput>(json!({
        "action": "pause",
        "expectedGoalId": "goal",
        "expectedRevision": 3
    }))
    .is_err());
}
