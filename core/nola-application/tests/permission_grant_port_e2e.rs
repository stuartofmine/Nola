// SPDX-License-Identifier: Apache-2.0

use nola_application::{
    execute_application_command, ApplicationCommand, PermissionGrant, PermissionGrantDecision,
    PermissionGrantInput, PermissionGrantTarget,
};
use nola_store::NolaStore;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn typed_host_grant_survives_application_service_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-grant-port.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let recorded = execute_application_command(
            &mut store,
            ApplicationCommand::UpsertPermissionGrant {
                input: PermissionGrantInput {
                    id: "host-write".into(),
                    target: PermissionGrantTarget::Host {
                        host_id: "mac-local".into(),
                    },
                    capability_pattern: "write_file".into(),
                    resource_pattern: "/workspace/*".into(),
                    decision: PermissionGrantDecision::Allow,
                    created_at_ms: 10,
                    updated_at_ms: 10,
                },
            },
        )
        .unwrap();
        let recorded: PermissionGrant = serde_json::from_value(recorded).unwrap();
        assert_eq!(
            recorded.target,
            PermissionGrantTarget::Host {
                host_id: "mac-local".into()
            }
        );
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let listed = execute_application_command(
        &mut reopened,
        ApplicationCommand::ListPermissionGrants {
            conversation_id: None,
            host_id: Some("mac-local".into()),
        },
    )
    .unwrap();
    let listed: Vec<PermissionGrant> = serde_json::from_value(listed).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].decision, PermissionGrantDecision::Allow);
}

#[test]
fn permission_grant_command_rejects_conflicting_store_target_fields() {
    let raw = json!({
        "type": "upsert_permission_grant",
        "input": {
            "id": "invalid",
            "target": {"type":"host","host_id":"mac-local"},
            "scope": "conversation",
            "conversation_id": "conversation-1",
            "host_id": "mac-local",
            "capability_pattern": "write_file",
            "resource_pattern": "*",
            "decision": "allow",
            "created_at_ms": 10,
            "updated_at_ms": 10
        }
    });
    assert!(serde_json::from_value::<ApplicationCommand>(raw).is_err());
}
