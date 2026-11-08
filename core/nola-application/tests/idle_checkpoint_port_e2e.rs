// SPDX-License-Identifier: Apache-2.0
//! Idle checkpoint, memory deltas and publication share a real isolated Store UoW.
#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{reduce_and_commit_idle_submission, ReduceAndCommitIdleSubmission};
use nola_agent_engine::agent::{AgentCoreConfig, AgentSubmission, SessionId};

fn store(path: &std::path::Path) -> nola_store::NolaStore {
    let mut store=nola_store::NolaStore::open(path).unwrap();
    store.create_conversation(&nola_store::CreateConversationInput {
        id:"idle".into(),title:"Idle fixture".into(),revision:1,settings_json:None,
        created_at_ms:1,updated_at_ms:1,
    }).unwrap();
    store
}

fn command(submission:AgentSubmission) -> ReduceAndCommitIdleSubmission {
    ReduceAndCommitIdleSubmission {
        conversation_id:"idle".into(),core_config:AgentCoreConfig {
            turn_identity: fixture_model_identity::identity(),
            session_id:Some(SessionId::from_string("idle".into())),..Default::default()
        },submission,committed_at_ms:10,
    }
}

fn usage() -> AgentSubmission {
    AgentSubmission::MemoryRuntime {
        envelope:nola_agent_engine::agent::MemoryRuntimeCommandEnvelope {
            operation_id:"usage-op".into(),command:nola_agent_engine::agent::MemoryRuntimeCommand::RecordUsage {
                entry:nola_agent_engine::agent::MemoryUsageLedgerEntry {
                    usage_id:"usage-id".into(),memory_id:"memory-id".into(),session_id:"idle".into(),
                    turn_id:"turn-id".into(),used_at_epoch_ms:10,context_payload_digest:"digest".into(),
                    citation_ids:vec!["citation".into()],
                },
            },
        },
    }
}

#[test]
fn idle_memory_usage_is_forwarded_once_with_the_checkpoint_delta() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("usage.sqlite");
    let mut facts=store(&path);
    facts.upsert_memory_record(&nola_store_port::memory::StoredMemoryRecord {
        id:"memory-id".into(),owner_json:r#"{"kind":"profile","profile_id":"local"}"#.into(),namespace_json:r#"{"kind":"personal"}"#.into(),scope:"global".into(),kind:"reference".into(),
        content:"A durable fixture fact".into(),summary:None,tags_json:"[]".into(),confidence:1.0,
        importance:0.5,sensitivity:None,retention_json:"{}".into(),expires_at:None,
        supersedes_json:"[]".into(),provenance_json:"[]".into(),created_at_ms:1,updated_at_ms:1,
        last_used_at_ms:None,deleted_at_ms:None,
    }).unwrap();
    let accepted = reduce_and_commit_idle_submission(&mut facts,command(usage())).unwrap();
    let wire = serde_json::to_value(&accepted).unwrap();
    assert!(wire.get("checkpoint").is_none());
    assert!(wire.get("snapshot").is_none());
    assert_eq!(wire["execution"]["control"]["session_id"], "idle");
    assert!(wire["execution"]["configuration"]["initial_items"].as_array().unwrap().is_empty());
    let db=rusqlite::Connection::open(&path).unwrap();
    let row:(String,String)=db.query_row("SELECT usage_id,citation_ids_json FROM memory_usage_ledger",[],
        |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(row.0,"usage-id");
    assert_eq!(serde_json::from_str::<Vec<String>>(&row.1).unwrap(),vec!["citation"]);
    assert_eq!(facts.load_checkpoint("idle").unwrap().unwrap().revision,1);
    // Even INSERT OR IGNORE would trigger this failure: repeated reduction
    // must emit no usage delta, rather than depend on database deduplication.
    db.execute_batch("CREATE TRIGGER forbid_repeated_usage BEFORE INSERT ON memory_usage_ledger
        BEGIN SELECT RAISE(ABORT,'usage delta was forwarded twice'); END;").unwrap();
    drop(facts);
    let mut reopened=nola_store::NolaStore::open(&path).unwrap();
    reduce_and_commit_idle_submission(&mut reopened,command(usage())).unwrap();
    assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,2);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM memory_usage_ledger",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM assistant_journal_records",[],|row|row.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn real_idle_core_commits_only_after_repository_success_and_restores() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("rollback.sqlite");
    let mut facts=store(&path);let db=rusqlite::Connection::open(&path).unwrap();
    let submission=||AgentSubmission::RebaseConversation {items:vec![],item_ownership:Default::default()};
    db.execute_batch("CREATE TRIGGER fail_checkpoint AFTER INSERT ON core_checkpoints
        BEGIN SELECT RAISE(ABORT,'checkpoint commit unavailable'); END;").unwrap();
    assert!(reduce_and_commit_idle_submission(&mut facts,command(submission())).unwrap_err()
        .contains("checkpoint commit unavailable"));
    assert!(facts.load_checkpoint("idle").unwrap().is_none());
    assert!(!facts.has_write_unit_of_work(),"failed reducer must close its owned transaction");
    drop(facts);
    let mut reopened=nola_store::NolaStore::open(&path).unwrap();
    assert!(reopened.load_checkpoint("idle").unwrap().is_none());
    assert_eq!(db.query_row("SELECT COUNT(*) FROM assistant_journal_publications",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    db.execute_batch("DROP TRIGGER fail_checkpoint;").unwrap();
    reduce_and_commit_idle_submission(&mut reopened,command(submission())).unwrap();
    assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,1);
    drop(reopened);
    let mut reopened=nola_store::NolaStore::open(&path).unwrap();
    reduce_and_commit_idle_submission(&mut reopened,command(submission())).unwrap();
    assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,2);
}
