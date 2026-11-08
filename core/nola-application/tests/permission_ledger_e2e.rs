// SPDX-License-Identifier: Apache-2.0
#[path = "support/model_identity.rs"]
mod fixture_model_identity;
#[path = "../../nola-store/tests/support/remote_submission.rs"]
mod remote_submission_fixture;
use remote_submission_fixture::RemoteSubmissionFixture;

use nola_application::load_permission_rule_set;
use nola_agent_engine::agent::permissions::{
    PermissionPolicyDecision, PermissionRule, PermissionRuleSet, PermissionRuleSource,
    PermissionSubject,
};
use nola_agent_engine::agent::{AgentCore, AgentCoreConfig, SessionId};
use nola_store::{
    AdmittedReplicaTurnInput, NolaStore, PermissionGrantScope, StoredPermissionDecision,
    UpsertPermissionGrantInput,
};
use tempfile::tempdir;

#[test]
fn application_projects_durable_permission_precedence_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-application.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .admit_remote_fixture(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "mac-local".into(),
                conversation_id: "permission-conversation".into(),
                conversation_title: "Permission".into(),
                user_message_id: "permission-user".into(),
                user_part_id: "permission-user:body".into(),
                user_text: "Run the task".into(),
                user_content_json: None,
                assistant_message_id: "permission-assistant".into(),
                assistant_part_id: "permission-assistant:body".into(),
                run_id: "permission-run".into(),
                context_checkpoint_id: None,
                created_at_ms: 1,
            })
            .unwrap();
        for (id, scope, conversation_id, host_id, decision, updated_at_ms) in [
            (
                "global",
                PermissionGrantScope::UserGlobal,
                None,
                None,
                StoredPermissionDecision::Deny,
                2,
            ),
            (
                "host",
                PermissionGrantScope::Host,
                None,
                Some("mac"),
                StoredPermissionDecision::Allow,
                3,
            ),
            (
                "conversation",
                PermissionGrantScope::Conversation,
                Some("permission-conversation"),
                None,
                StoredPermissionDecision::Deny,
                4,
            ),
        ] {
            store
                .upsert_permission_grant(&UpsertPermissionGrantInput {
                    id: id.into(),
                    scope,
                    conversation_id: conversation_id.map(str::to_string),
                    host_id: host_id.map(str::to_string),
                    capability_pattern: "write_file".into(),
                    resource_pattern: if scope == PermissionGrantScope::Conversation {
                        r#"{"path":"/workspace/result.txt"}"#.into()
                    } else {"*".into()},
                    decision,
                    created_at_ms: updated_at_ms,
                    updated_at_ms,
                })
                .unwrap();
        }
    }
    let store = NolaStore::open(&database).unwrap();
    let subject = PermissionSubject {
        capability: "write_file".into(),
        resource: r#"{"path":"/workspace/result.txt"}"#.into(),
    };
    let conversation_rules =
        load_permission_rule_set(&store, Some("permission-conversation"), Some("mac")).unwrap();
    assert_eq!(
        conversation_rules.evaluate(&subject),
        PermissionPolicyDecision::Deny
    );

    let host_rules = load_permission_rule_set(&store, None, Some("mac")).unwrap();
    assert_eq!(
        host_rules.evaluate(&subject),
        PermissionPolicyDecision::Ask
    );
    assert_eq!(
        host_rules.evaluate_resolved_for_origin_with_explanation(&subject,
            nola_agent_engine::agent::protocol::AgentInputOrigin::InteractiveUser,false).decision,
        PermissionPolicyDecision::Allow
    );

    let global_rules = load_permission_rule_set(&store, None, Some("linux")).unwrap();
    assert_eq!(
        global_rules.evaluate(&subject),
        PermissionPolicyDecision::Deny
    );
}

#[test]
fn remote_run_and_retained_legacy_bytes_never_grant_local_authority() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-mirror.db");
    let conversation_id = "permission-mirror-conversation";
    let snapshot = AgentCore::try_new(AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string(conversation_id.to_string())),
        permission_rules: PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "write_file".into(),
                resource_pattern: r#"{"path":"/workspace/*"}"#.into(),
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        },
        ..AgentCoreConfig::default()
    })
    .unwrap()
    .snapshot();
    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .admit_remote_fixture(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "mac-local".into(),
                conversation_id: conversation_id.into(),
                conversation_title: "Permission mirror".into(),
                user_message_id: "permission-mirror-user".into(),
                user_part_id: "permission-mirror-user:body".into(),
                user_text: "Edit the file".into(),
                user_content_json: None,
                assistant_message_id: "permission-mirror-assistant".into(),
                assistant_part_id: "permission-mirror-assistant:body".into(),
                run_id: "permission-mirror-run".into(),
                context_checkpoint_id: None,
                created_at_ms: 1,
            })
            .unwrap();

    }

    // Historical bytes may remain on disk, but no retired mirror reader may
    // turn them into execution or permission authority.
    let connection=rusqlite::Connection::open(&database).unwrap();
    connection.execute("INSERT INTO remote_session_mirrors(conversation_id,source_host_id,core_revision,event_cursor,snapshot_json,updated_at_ms,run_id) VALUES(?1,'mac-local',0,0,?2,200,'permission-mirror-run')",rusqlite::params![conversation_id,serde_json::to_string(&snapshot).unwrap()]).unwrap();
    drop(connection);
    let store = NolaStore::open(&database).unwrap();
    let rules = load_permission_rule_set(&store, Some(conversation_id), Some("mac-local")).unwrap();
    assert!(rules.rules.is_empty());
    assert_ne!(
        rules.evaluate(&PermissionSubject {
            capability: "write_file".into(),
            resource: r#"{"path":"/workspace/report.md"}"#.into(),
        }),
        PermissionPolicyDecision::Allow
    );
    assert!(nola_application::load_session_projection(&store, conversation_id).unwrap().is_none());
}
