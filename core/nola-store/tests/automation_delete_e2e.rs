// SPDX-License-Identifier: Apache-2.0

use nola_store::{NolaStore, StoredAutomationDefinition};
use std::path::PathBuf;

fn database_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "nola-automation-delete-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn deleting_an_automation_is_atomic_idempotent_and_survives_reopen() {
    let database = database_path();
    {
        let mut store = NolaStore::open(&database).expect("open Store");
        store
            .upsert_automation_definition(&StoredAutomationDefinition {
                execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
                id: "daily-review".into(),
                name: "Daily review".into(),
                schedule_format: "manual".into(),
                enabled: true,
                revision: 99,
                metadata_json: "{}".into(),
                created_at_ms: 1_000,
                updated_at_ms: 1_000,
                deleted_at_ms: None,
            })
            .expect("save automation");

        assert!(store
            .delete_automation_definition("daily-review", 2_000)
            .expect("delete automation"));
        assert!(!store
            .delete_automation_definition("daily-review", 3_000)
            .expect("repeat delete"));
        assert!(store
            .list_automation_definitions()
            .expect("list active automations")
            .is_empty());
    }

    let reopened = NolaStore::open(&database).expect("reopen Store");
    assert!(reopened
        .list_automation_definitions()
        .expect("list after reopen")
        .is_empty());
    drop(reopened);
    let _ = std::fs::remove_file(database);
}
