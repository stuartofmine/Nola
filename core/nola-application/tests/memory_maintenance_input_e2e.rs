#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{
    execute_application_command, reduce_and_commit_idle_submission, ApplicationCommand,
    MemoryCandidate, MemoryCommand, MemoryCommitIntent, ReduceAndCommitIdleSubmission,
};
use nola_agent_engine::agent::memory_runtime::{
    consolidation_input_digest, MemoryConsolidationOutput, MemoryJobOutput,
    MemoryRuntimeCommandEnvelope,
};
use nola_agent_engine::agent::{AgentCoreConfig, AgentSubmission, MemoryRuntimeCommand};
use nola_store::{CreateConversationInput, NolaStore};

fn candidate(content: &str, time: i64) -> MemoryCandidate {
    serde_json::from_value(serde_json::json!({
        "record": {"id": "input", "owner": {"kind":"profile","profile_id":"local"}, "namespace": {"kind": "personal"}, "scope": "global",
            "kind": "preference", "content": content, "summary": "Response style", "tags": [],
            "confidence": 0.9, "importance": 0.8, "sensitivity": null,
            "retention": {"mode": "until_superseded"}, "supersedes": [],
            "provenance": {"kind": "explicit_user", "source": "maintenance-test",
                "session_id": null, "turn_id": null, "item_ids": [], "observed_at": null}},
        "quality": null, "proposed_at_ms": time
    }))
    .unwrap()
}

fn submit(store: &mut NolaStore, submission: AgentSubmission) -> Result<(), String> {
    reduce_and_commit_idle_submission(
        store,
        ReduceAndCommitIdleSubmission {
            conversation_id: "maintenance".into(),
            core_config: AgentCoreConfig {
                turn_identity: fixture_model_identity::identity(),
                session_id: Some(nola_agent_engine::agent::SessionId::from_string(
                    "maintenance".into(),
                )),
                ..AgentCoreConfig::default()
            },
            submission,
            committed_at_ms: 500,
        },
    )
    .map(|_| ())
}

fn write(store: &mut NolaStore, content: &str, time: i64) {
    execute_application_command(
        store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: candidate(content, time),
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();
}

fn setup(store: &mut NolaStore) {
    store
        .create_conversation(&CreateConversationInput {
            id: "maintenance".into(),
            title: "Maintenance".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    write(store, "Prefer concise replies", 100);
    let record = candidate("Prefer concise replies", 100).record;
    let digest = consolidation_input_digest(&[record.clone()]).unwrap();
    submit(store, AgentSubmission::CreateMemory { record }).unwrap();
    submit(
        store,
        AgentSubmission::MemoryRuntime {
            envelope: MemoryRuntimeCommandEnvelope {
                operation_id: "create-job".into(),
                command: MemoryRuntimeCommand::CreateConsolidation {
                    job_id: "job".into(),
                    input_watermark: "input-v1".into(),
                    input_digest: digest,
                    selected_memory_ids: vec!["input".into()],
                    now_epoch_ms: 200,
                },
            },
        },
    )
    .unwrap();
    submit(
        store,
        AgentSubmission::ClaimMemoryWorker {
            operation_id: "claim-job".into(),
            job_id: "job".into(),
            worker_id: "worker".into(),
            ownership_token: "lease".into(),
            now_epoch_ms: 201,
            lease_until_epoch_ms: 1000,
        },
    )
    .unwrap();
}

fn completion(direct: bool) -> AgentSubmission {
    let output = MemoryJobOutput::Consolidation(MemoryConsolidationOutput {
        records: vec![],
        summary: "No additional durable facts".into(),
        output_digest: "result-v1".into(),
        citations: vec![],
        superseded_memory_ids: vec![],
        quality_by_memory_id: Default::default(),
    });
    if direct {
        AgentSubmission::MemoryRuntime {
            envelope: MemoryRuntimeCommandEnvelope {
                operation_id: "complete-job".into(),
                command: MemoryRuntimeCommand::Complete {
                    job_id: "job".into(),
                    ownership_token: "lease".into(),
                    now_epoch_ms: 500,
                    completion_watermark: "input-v1".into(),
                    output,
                },
            },
        }
    } else {
        AgentSubmission::MemoryWorkerCompleted {
            operation_id: "complete-job".into(),
            job_id: "job".into(),
            ownership_token: "lease".into(),
            now_epoch_ms: 500,
            completion_watermark: "input-v1".into(),
            output: serde_json::to_value(output).unwrap(),
        }
    }
}

#[test]
fn edited_or_deleted_input_rejects_both_completion_paths_after_restart() {
    for direct in [false, true] {
        for deleted in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("maintenance.db");
            let mut store = NolaStore::open(&path).unwrap();
            setup(&mut store);
            if deleted {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::Memory {
                        command: MemoryCommand::Delete {
                            id: "input".into(),
                            deleted_at_ms: 300,
                        },
                    },
                )
                .unwrap();
            } else {
                write(&mut store, "Prefer detailed replies", 300);
            }
            let before = store.load_checkpoint("maintenance").unwrap().unwrap();
            drop(store);
            let mut store = NolaStore::open(&path).unwrap();
            let error = submit(&mut store, completion(direct)).unwrap_err();
            assert!(error.contains("memory_input_conflict"), "{error}");
            drop(store);
            let store = NolaStore::open(&path).unwrap();
            let after = store.load_checkpoint("maintenance").unwrap().unwrap();
            assert_eq!(before.revision, after.revision);
            assert_eq!(before.checkpoint_json, after.checkpoint_json);
            let record = store.memory_record("input").unwrap();
            if deleted {
                assert!(record.is_none());
            } else {
                assert_eq!(record.unwrap().content, "Prefer detailed replies");
            }
        }
    }
}

#[test]
fn unchanged_input_can_complete_through_both_paths() {
    for direct in [false, true] {
        let mut store = NolaStore::open(":memory:").unwrap();
        setup(&mut store);
        submit(&mut store, completion(direct)).unwrap();
        let checkpoint = store.load_checkpoint("maintenance").unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
        assert_eq!(
            json.pointer("/state/memory_runtime/jobs/job/state")
                .unwrap(),
            "completed"
        );
    }
}
