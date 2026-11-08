#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_agent_engine::agent::recovery::CheckpointReason;
use nola_agent_engine::agent::*;
use nola_store::{
    work_archive::read_verified, CreateConversationInput, NolaStore, SaveCheckpointInput,
};
use nola_store_port::run_effect::*;

fn submit(core: &mut AgentCore, id: &str, command: GoalRuntimeCommand) {
    core.submit(AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: id.into(),
            command,
        },
    })
    .unwrap();
}

#[test]
fn archived_goal_can_be_read_without_source_database_or_restoring_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("original.db");
    let config = AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("archive-session".into())),
        ..Default::default()
    };
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    submit(
        &mut core,
        "start",
        GoalRuntimeCommand::Start {
            id: "g".into(),
            objective: "完成修复".into(),
            constraints: vec!["不得删除用户数据".into()],
            requirements: vec![GoalRequirement {
                acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1, id: "r".into(),
                description: "验证通过".into(),
                dependencies: vec![],
                status: GoalRecordStatus::Pending,
                evidence_refs: vec![],
            }],
            now_epoch_ms: 1,
        },
    );
    let checkpoint = core.checkpoint(CheckpointReason::GoalUpdated);
    let mut store = NolaStore::open(&db).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "archive-session".into(),
            title: "test".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "archive-session".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: checkpoint.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            updated_at_ms: 1,
        })
        .unwrap();
    let directory = store.work_archive_directory("archive-session").unwrap();
    // Export completion is explicit; Store drop only stops scheduling and must
    // not be mistaken for a barrier that finished the asynchronous publisher.
    store.flush_work_archives().unwrap();
    let publication = store.archive_publication_status().unwrap();
    assert_eq!(publication.published_generation, publication.target_generation);
    read_verified(&directory).unwrap();
    drop(store);
    std::fs::rename(&db, temp.path().join("offline.db")).unwrap();
    let archive = read_verified(&directory).unwrap();
    let restored =
        nola_application::inspect_archived_work(config.clone(), &archive.recovery).unwrap();
    assert_eq!(
        restored["goal"]["active"]["constraints"][0],
        "不得删除用户数据"
    );
    assert_eq!(restored["execution_resumed"], false);
    assert_eq!(restored["requires_permission_revalidation"], true);

    // A baseline alone is not current: replay the exact committed submission.
    let pause = AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: "pause".into(),
            command: GoalRuntimeCommand::Pause {
                status: GoalStatus::Paused,
                wake_at_epoch_ms: None,
                now_epoch_ms: 2,
            },
        },
    };
    core.submit(pause.clone()).unwrap();
    let mut recovery = archive.recovery;
    recovery.checkpoint.revision = 2;
    recovery.checkpoint.event_cursor = core
        .checkpoint(CheckpointReason::GoalUpdated)
        .last_event_cursor
        .sequence as i64;
    recovery.replay_base = Some(StoredCheckpointReplayBase {
        run_id: "run".into(),
        checkpoint_revision: 1,
        through_ordinal: 0,
    });
    recovery.replay_inputs = vec![StoredCoreReplayInput {
        kind: ReplayInputKind::Submission,
        ordinal: 1,
        received_at_ms: 2,
        reduction_json: serde_json::to_string(&SubmissionReplayReduction {
            protocol_version: 1,
            session_id: "archive-session".into(),
            decision_time_ms: 2,
            submission_json: serde_json::to_string(&pause).unwrap(),
            bindings: vec![],
        })
        .unwrap(),
    }];
    let restored = nola_application::inspect_archived_work(config.clone(), &recovery).unwrap();
    assert_eq!(restored["goal"]["active"]["status"], "paused");
    recovery.replay_inputs[0].ordinal = 3;
    assert!(nola_application::inspect_archived_work(config, &recovery).is_err());
}
