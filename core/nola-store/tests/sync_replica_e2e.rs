use nola_store::NolaStore;
use tempfile::tempdir;

#[test]
fn product_setting_sync_outbox_and_cursor_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("canonical.db");
    let operation_id;
    {
        let mut store = NolaStore::open(&path).unwrap();
        let first = store
            .put_product_setting("assistant.memory.enabled", r#"{"enabled":true}"#, 10)
            .unwrap();
        assert_eq!(first.revision, 1);
        let pending = store.pending_sync_changes(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entity_type, "product_setting");
        assert_eq!(pending[0].entity_id, "assistant.memory.enabled");
        assert!(!pending[0].deleted);
        operation_id = pending[0].operation_id.clone();
    }
    {
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            nola_store::STORE_SCHEMA_VERSION
        );
        assert_eq!(
            store.pending_sync_changes(10).unwrap()[0].operation_id,
            operation_id
        );
        assert!(store
            .resolve_sync_change(&operation_id, "acknowledged", Some(41), 20)
            .unwrap());
        store.save_sync_cursor("primary", 41, 20).unwrap();
        assert!(store
            .delete_product_setting("assistant.memory.enabled")
            .unwrap());
        let pending = store.pending_sync_changes(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].revision, 2);
        assert!(pending[0].deleted);
        assert_eq!(pending[0].payload_json, "null");
    }
    {
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.sync_cursor("primary").unwrap().unwrap().cursor, 41);
        assert_eq!(store.pending_sync_changes(10).unwrap().len(), 1);
        assert!(store
            .product_setting("assistant.memory.enabled")
            .unwrap()
            .is_none());
    }
}
