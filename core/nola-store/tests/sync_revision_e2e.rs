use nola_store::{NolaStore, STORE_SCHEMA_VERSION};
use tempfile::tempdir;

#[test]
fn deleted_product_setting_recreation_continues_revision_after_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        assert_eq!(
            store
                .put_product_setting("assistant.memory.enabled", "true", 1_000)
                .unwrap()
                .revision,
            1
        );
        assert!(store
            .delete_product_setting("assistant.memory.enabled")
            .unwrap());
    }

    {
        let mut reopened = NolaStore::open(&database).unwrap();
        let recreated = reopened
            .put_product_setting("assistant.memory.enabled", "false", 2_000)
            .unwrap();
        assert_eq!(recreated.revision, 3);
        let changes = reopened.pending_sync_changes(10).unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.revision, change.deleted))
                .collect::<Vec<_>>(),
            vec![(1, false), (2, true), (3, false)]
        );
    }
}
