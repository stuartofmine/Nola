use nola_store::{CreateConversationInput, NolaStore, StoredDataSourceRecord, StoredDataSourceUsage};

fn source(updated_at_ms: i64) -> StoredDataSourceRecord {
    StoredDataSourceRecord {
        id: "sales".into(),
        name: "Sales data".into(),
        kind: "localFile".into(),
        location: "/data/sales.csv".into(),
        enabled: true,
        access_policy: "conversationOptIn".into(),
        validation_contract_json:
            r#"{"readRecipe":{"method":"localFileRead"},"redactedSample":"CSV · 2 rows"}"#.into(),
        capabilities_json: r#"["read","context"]"#.into(),
        privacy: "local-first".into(),
        metadata_json: r#"{"maximumBytes":262144}"#.into(),
        revision: 0,
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    }
}

#[test]
fn verified_source_override_and_usage_survive_cold_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    let first = store.upsert_data_source(&source(100)).expect("save source");
    assert_eq!(first.revision, 1);
    let unchanged = store
        .upsert_data_source(&source(200))
        .expect("replay source");
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.updated_at_ms, 100);
    assert!(unchanged.enabled);
    assert_eq!(unchanged.metadata_json, r#"{"maximumBytes":262144}"#);

    store.create_conversation(&CreateConversationInput {
        id: "conversation-1".into(), title: "Source test".into(), revision: 1,
        settings_json: None, created_at_ms: 100, updated_at_ms: 100,
    }).expect("create conversation");

    store
        .set_data_source_override("conversation-1", "sales", Some(true), 300)
        .expect("save override");
    let usage = StoredDataSourceUsage {
        id: "usage-1".into(),
        source_id: "sales".into(),
        source_title: "Sales data".into(),
        consumer: "chat".into(),
        purpose: "conversation_context".into(),
        conversation_id: Some("conversation-1".into()),
        disclosed_to_cloud: None,
        metadata_json: r#"{"model":"cloud-model"}"#.into(),
        used_at_ms: 400,
    };
    store.append_data_source_usage(&usage).expect("save usage");
    store
        .append_data_source_usage(&usage)
        .expect("replay usage");
    drop(store);

    let mut reopened = NolaStore::open(&path).expect("reopen");
    assert_eq!(reopened.list_data_sources().unwrap().len(), 1);
    assert_eq!(
        reopened
            .data_source_override("conversation-1", "sales")
            .unwrap(),
        Some(true)
    );
    assert_eq!(reopened.list_data_source_usage(20).unwrap(), vec![usage]);
    assert!(reopened.delete_data_source("sales", 500).unwrap());
    assert!(reopened.data_source("sales").unwrap().is_none());
    assert_eq!(
        reopened
            .data_source_override("conversation-1", "sales")
            .unwrap(),
        None
    );
    assert_eq!(reopened.list_data_source_usage(20).unwrap().len(), 1);
}

#[test]
fn override_requires_a_live_conversation_and_is_cleared_on_delete() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut store = NolaStore::open(temp.path().join("nola.sqlite")).expect("open");
    store.upsert_data_source(&source(100)).expect("source");
    assert!(store.set_data_source_override("missing", "sales", Some(true), 101).is_err());
    assert_eq!(store.data_source_override("missing", "sales").unwrap(), None);
    store.create_conversation(&CreateConversationInput {
        id: "conversation-1".into(), title: "Source test".into(), revision: 1,
        settings_json: None, created_at_ms: 100, updated_at_ms: 100,
    }).expect("create conversation");
    store.set_data_source_override("conversation-1", "sales", Some(true), 101).expect("override");
    assert!(store.delete_conversation("conversation-1").expect("delete"));
    assert_eq!(store.data_source_override("conversation-1", "sales").unwrap(), None);
    assert!(store.set_data_source_override("conversation-1", "sales", None, 102).is_err());
}

#[test]
fn invalid_contract_is_rejected_without_partial_fact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    let mut invalid = source(100);
    invalid.validation_contract_json = "raw secret, not JSON".into();
    assert!(store.upsert_data_source(&invalid).is_err());
    assert!(store.list_data_sources().unwrap().is_empty());
}
