use nola_store::{NolaStore, StoredAutomationDefinition};

#[test]
fn conditional_automation_update_rejects_stale_and_deleted_revisions_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("automation.sqlite");
    let mut first = NolaStore::open(&path).unwrap();
    let mut second = NolaStore::open(&path).unwrap();
    let original = first.upsert_automation_definition(&StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id:"reviewed".into(), name:"Script".into(), schedule_format:"script".into(), enabled:false,
        revision:0, metadata_json:r#"{"script":"old","approved":false}"#.into(), created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
    }).unwrap();
    let mut edited = original.clone(); edited.metadata_json = r#"{"script":"changed","approved":false}"#.into(); edited.updated_at_ms = 2;
    let current = second.upsert_automation_definition(&edited).unwrap();
    let outbox_count = first.pending_sync_changes(100).unwrap().len();
    let mut approval = original.clone(); approval.enabled = true; approval.metadata_json = r#"{"script":"old","approved":true}"#.into(); approval.updated_at_ms = 3;
    assert!(first.update_automation_definition_if_revision(&approval, original.revision).unwrap().is_none());
    assert_eq!(first.automation_definition("reviewed").unwrap().unwrap(), current);
    assert_eq!(first.pending_sync_changes(100).unwrap().len(), outbox_count);
    let mut foreign = current.clone(); foreign.schedule_format = "nola.mac.automation.v1".into();
    assert!(first.update_automation_definition_if_revision(&foreign, current.revision).unwrap().is_none());
    assert_eq!(first.pending_sync_changes(100).unwrap().len(), outbox_count);
    let mut reviewed = current.clone(); reviewed.enabled = true;
    let saved = first.update_automation_definition_if_revision(&reviewed, current.revision).unwrap().unwrap();
    assert_eq!(saved.revision, current.revision + 1);
    assert!(second.update_automation_definition_if_revision(&reviewed, current.revision).unwrap().is_none());
    first.delete_automation_definition("reviewed", 10).unwrap();
    assert!(second.update_automation_definition_if_revision(&reviewed, saved.revision).unwrap().is_none());
    assert!(second.automation_definition("reviewed").unwrap().is_none());
}

#[test]
fn only_one_concurrent_writer_can_commit_a_reviewed_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("race.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let reviewed = store.upsert_automation_definition(&StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id:"race".into(), name:"Script".into(), schedule_format:"script".into(), enabled:false,
        revision:0, metadata_json:"{}".into(), created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
    }).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2).map(|index| {
        let path = path.clone(); let barrier = barrier.clone(); let mut input = reviewed.clone();
        std::thread::spawn(move || {
            let mut connection = NolaStore::open(path).unwrap();
            input.metadata_json = format!("{{\"writer\":{index}}}");
            barrier.wait();
            connection.update_automation_definition_if_revision(&input, input.revision).unwrap().is_some()
        })
    }).collect::<Vec<_>>();
    assert_eq!(workers.into_iter().map(|worker| usize::from(worker.join().unwrap())).sum::<usize>(), 1);
    assert_eq!(store.automation_definition("race").unwrap().unwrap().revision, reviewed.revision + 1);
}
