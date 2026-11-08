use nola_store::{NolaStore, StoredAutomationDefinition};

fn definition(id: &str, name: &str, updated_at_ms: i64) -> StoredAutomationDefinition {
    StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id: id.into(),
        name: name.into(),
        schedule_format: "cron".into(),
        enabled: true,
        revision: 0,
        metadata_json: format!(r#"{{"id":"{id}","name":"{name}","format":"cron"}}"#),
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    }
}

#[test]
fn scoped_reconcile_preserves_other_formats_and_rejects_identity_takeover_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temp.path().join("scope.sqlite")).unwrap();
    let mut mac = definition("mac", "Mac task", 100);
    mac.schedule_format = "nola.mac.automation.v1".into();
    let committed_mac = store.upsert_automation_definition(&mac).unwrap();
    let web = definition("web", "Web task", 100);
    store.upsert_automation_definition(&web).unwrap();
    let scope = ["cron".to_string()];

    let result = store.reconcile_automation_definitions(&[web.clone()], &scope, 200).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "web");
    assert_eq!(store.automation_definition("mac").unwrap(), Some(committed_mac.clone()));
    let before = store.list_automation_definitions().unwrap();
    let mut changed_web = web.clone(); changed_web.name = "Must roll back".into();
    let collision = definition("mac", "Foreign overwrite", 300);
    assert!(store.reconcile_automation_definitions(&[changed_web, collision], &scope, 300).is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before);
    assert!(store.reconcile_automation_definitions(&[mac], &scope, 300).is_err());
    assert!(store.reconcile_automation_definitions(&[], &[], 300).is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before);

    assert!(store.reconcile_automation_definitions(&[], &scope, 400).unwrap().is_empty());
    assert!(store.automation_definition("web").unwrap().is_none());
    assert_eq!(store.automation_definition("mac").unwrap(), Some(committed_mac));
    assert!(!store.pending_sync_changes(100).unwrap().iter().any(|change| change.entity_id == "mac" && change.deleted));
}

#[test]
fn automation_reconcile_is_atomic_versioned_and_survives_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");

    let first = store
        .upsert_automation_definition(&definition("daily", "Daily brief", 100))
        .expect("insert");
    assert_eq!(first.revision, 1);
    let unchanged = store
        .upsert_automation_definition(&definition("daily", "Daily brief", 200))
        .expect("idempotent upsert");
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.updated_at_ms, 100);

    let reconciled = store
        .reconcile_automation_definitions(
            &[
                definition("daily", "Daily briefing", 300),
                definition("weekly", "Weekly review", 300),
            ],
            &["cron".into()],
            300,
        )
        .expect("reconcile");
    assert_eq!(reconciled.len(), 2);
    assert_eq!(
        store
            .automation_definition("daily")
            .unwrap()
            .unwrap()
            .revision,
        2
    );

    store
        .reconcile_automation_definitions(&[definition("weekly", "Weekly review", 300)], &["cron".into()], 400)
        .expect("remove missing definition");
    assert!(store.automation_definition("daily").unwrap().is_none());

    let before_invalid = store.list_automation_definitions().unwrap();
    let mut invalid = definition("broken", "", 500);
    invalid.metadata_json = "not-json".into();
    assert!(store
        .reconcile_automation_definitions(&[invalid], &["cron".into()], 500)
        .is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before_invalid);

    drop(store);
    let mut reopened = NolaStore::open(&path).expect("reopen");
    assert_eq!(
        reopened.list_automation_definitions().unwrap(),
        before_invalid
    );
    let restored = reopened
        .reconcile_automation_definitions(
            &[
                definition("daily", "Daily briefing", 600),
                definition("weekly", "Weekly review", 300),
            ],
            &["cron".into()],
            600,
        )
        .expect("restore");
    assert_eq!(restored.len(), 2);
    assert_eq!(
        reopened
            .automation_definition("daily")
            .unwrap()
            .unwrap()
            .revision,
        4
    );
}
