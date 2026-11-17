// SPDX-License-Identifier: Apache-2.0
//! Consolidated Permission Lifecycle suite:
//! non-SQLite repository & wire boundaries, configuration snapshots & grants,
//! durable precedence ledger & remote mirror isolation, selective permission
//! projection reads, and runtime task permission request arbitration/receipts.

use crate::remote_submission as remote_submission_fixture;
use crate::support;
use crate::support::admission_frontier;
use crate::support::model_identity as fixture_model_identity;
use crate::terminal_sources;
use nola_agent_engine::agent::permissions::{
    PermissionPolicyDecision, PermissionRule, PermissionRuleSet, PermissionRuleSource,
    PermissionSubject,
};
use nola_agent_engine::agent::{AgentCore, AgentCoreConfig, SessionId};
use nola_application::conversation_configuration::SelectConversationExecutionInput;
use nola_application::permission::execute_permission_request_command;
use nola_application::{
    execute_application_command, load_permission_rule_set, ApplicationCommand,
    ApplicationCommandReceiptOutcome, CreateRuntimeTask, ExecuteReceivedCommand,
    NewPermissionRequest, PermissionAuthorizationDecision, PermissionGrant,
    PermissionGrantDecision, PermissionGrantInput, PermissionGrantTarget, PermissionIntervention,
    PermissionProjectionReadScope, PermissionRequest, PermissionRequestCommand,
    PermissionRequestResolution, PermissionRequestStatus, PermissionResolutionOutcome, RuntimeTask,
    RuntimeTaskCommand, RuntimeTaskLeaseWitness, RuntimeTaskPriority, RuntimeTaskResourceBudget,
    RuntimeTaskResourceUsage, RuntimeTaskScheduling, RuntimeTaskTerminalSource,
};
use nola_host_protocol::PermissionInstruction;
use nola_store::{
    AdmittedReplicaTurnInput, BeginTurnInput, CreateConversationInput, NolaStore,
    PermissionGrantScope, StoredPermissionDecision, UpsertPermissionGrantInput,
};
use nola_store_port::conversation_query::CurrentPermissionState;
use nola_store_port::permission::{
    PermissionRepository, StoredPermissionGrant, StoredPermissionRequest,
    StoredPermissionResolutionOutcome,
};
use remote_submission_fixture::RemoteSubmissionFixture;
use serde_json::{json, Value};
use tempfile::tempdir;

#[derive(Default)]
struct NonSqlitePermissionFacts {
    request: Option<StoredPermissionRequest>,
    grants: Vec<StoredPermissionGrant>,
    fail: bool,
}
impl PermissionRepository for NonSqlitePermissionFacts {
    type Error = &'static str;
    fn ensure_core_permission_request(
        &mut self,
        _: &StoredPermissionRequest,
    ) -> Result<StoredPermissionRequest, Self::Error> {
        Err("fixture operation unavailable")
    }
    fn enqueue_permission_request(
        &mut self,
        r: &StoredPermissionRequest,
    ) -> Result<(), Self::Error> {
        self.request = Some(r.clone());
        Ok(())
    }
    fn permission_request(&self, _: &str) -> Result<Option<StoredPermissionRequest>, Self::Error> {
        Ok(self.request.clone())
    }
    fn pending_permission_for_task(
        &self,
        _: &str,
    ) -> Result<Option<StoredPermissionRequest>, Self::Error> {
        unimplemented!()
    }
    fn pending_permissions_for_owner(
        &self,
        _: &str,
    ) -> Result<Vec<StoredPermissionRequest>, Self::Error> {
        unimplemented!()
    }
    fn resolve_permission_request(
        &mut self,
        request_id: &str,
        decision: &str,
        response: Option<&str>,
        at: i64,
        grant: Option<&UpsertPermissionGrantInput>,
    ) -> Result<StoredPermissionResolutionOutcome, Self::Error> {
        if self.fail {
            return Err("repository unavailable");
        }
        let Some(r) = self.request.as_mut().filter(|r| r.request_id == request_id) else {
            return Ok(StoredPermissionResolutionOutcome::Missing {
                request_id: request_id.into(),
            });
        };
        if r.status != "pending" {
            return Ok(
                if r.decision.as_deref() == Some(decision)
                    && r.custom_response.as_deref() == response
                {
                    StoredPermissionResolutionOutcome::AlreadyResolved(r.clone())
                } else {
                    StoredPermissionResolutionOutcome::Conflict(r.clone())
                },
            );
        }
        assert!(grant.is_none());
        r.status = "resolved".into();
        r.decision = Some(decision.into());
        r.custom_response = response.map(str::to_owned);
        r.resolved_at_ms = Some(at);
        Ok(StoredPermissionResolutionOutcome::Resolved(r.clone()))
    }
    fn upsert_permission_grant(
        &mut self,
        _: &UpsertPermissionGrantInput,
    ) -> Result<StoredPermissionGrant, Self::Error> {
        unimplemented!()
    }
    fn revoke_permission_grant(&mut self, _: &str, _: i64) -> Result<bool, Self::Error> {
        unimplemented!()
    }
    fn effective_permission_grants(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<StoredPermissionGrant>, Self::Error> {
        if self.fail {
            Err("repository unavailable")
        } else {
            Ok(self.grants.clone())
        }
    }
}

#[test]
fn non_sqlite_permission_repository_wire_schemas_and_execution_selection_contracts() {
    // 1. Non-SQLite intervention & ordered grant evaluation.
    let mut facts = NonSqlitePermissionFacts::default();
    execute_permission_request_command(
        &mut facts,
        PermissionRequestCommand::Enqueue {
            input: NewPermissionRequest {
                core_permission_request_id: Some("core-fixture-request".into()),
                permission_instructions: Vec::new(),
                request_id: "r".into(),
                task_id: "t".into(),
                child_session_id: "c".into(),
                capability: "write_file".into(),
                input: json!({"path":"结果.txt"}),
                scope_rule: "*".into(),
                created_at_ms: 1,
            },
        },
    )
    .unwrap();
    let intervene = PermissionRequestCommand::Intervene {
        request_id: "r".into(),
        intervention: PermissionIntervention::ProvideInstruction {
            instruction: "  请只修改注释  ".into(),
        },
        resolved_at_ms: 2,
    };
    facts.fail = true;
    assert!(
        execute_permission_request_command(&mut facts, intervene.clone())
            .unwrap_err()
            .contains("repository unavailable")
    );
    assert_eq!(facts.request.as_ref().unwrap().status, "pending");
    facts.fail = false;
    assert_eq!(
        execute_permission_request_command(&mut facts, intervene.clone()).unwrap()["type"],
        json!("resolved")
    );
    assert_eq!(
        execute_permission_request_command(&mut facts, intervene).unwrap()["type"],
        json!("already_resolved")
    );
    let req: PermissionRequest = serde_json::from_value(
        execute_permission_request_command(
            &mut facts,
            PermissionRequestCommand::Get {
                request_id: "r".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(req.input, json!({"path":"结果.txt"}));
    assert_eq!(
        req.resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "请只修改注释".into()
            }
        })
    );
    facts.request.as_mut().unwrap().status = "unknown".into();
    assert!(execute_permission_request_command(
        &mut facts,
        PermissionRequestCommand::Get {
            request_id: "r".into()
        }
    )
    .is_err());

    for (scope, decision) in [
        (PermissionGrantScope::UserGlobal, StoredPermissionDecision::Deny),
        (PermissionGrantScope::Host, StoredPermissionDecision::Allow),
        (PermissionGrantScope::Conversation, StoredPermissionDecision::Deny),
    ] {
        facts.grants.push(StoredPermissionGrant {
            id: format!("{scope:?}"),
            scope,
            conversation_id: None,
            host_id: None,
            capability_pattern: "write_file".into(),
            resource_pattern: if scope == PermissionGrantScope::Conversation {
                "结果.txt".into()
            } else {
                "*".into()
            },
            decision,
            created_at_ms: 1,
            updated_at_ms: 1,
            revoked_at_ms: None,
        });
    }
    let subject = PermissionSubject {
        capability: "write_file".into(),
        resource: "结果.txt".into(),
    };
    assert_eq!(
        load_permission_rule_set(&facts, None, None)
            .unwrap()
            .evaluate(&subject),
        PermissionPolicyDecision::Deny
    );
    facts.grants.pop();
    assert_eq!(
        load_permission_rule_set(&facts, None, None)
            .unwrap()
            .evaluate(&subject),
        PermissionPolicyDecision::Ask
    );
    assert_eq!(
        load_permission_rule_set(&facts, None, None)
            .unwrap()
            .evaluate_resolved_for_origin_with_explanation(
                &subject,
                nola_agent_engine::agent::protocol::AgentInputOrigin::InteractiveUser,
                false
            )
            .decision,
        PermissionPolicyDecision::Allow
    );
    facts.fail = true;
    assert!(load_permission_rule_set(&facts, None, None)
        .unwrap_err()
        .contains("repository unavailable"));

    // 2. Wire rejections: conflicting grant target, bulk task denial, persistence input_json, and exclusive selection.
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
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
    }))
    .is_err());

    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type":"permission_request",
        "command":{"type":"deny_pending_for_task","task_id":"task-1","resolved_at_ms":30}
    }))
    .is_err());
    assert!(serde_json::from_value::<PermissionRequestCommand>(json!({
        "type":"deny_pending_for_task","task_id":"task-1","resolved_at_ms":30
    }))
    .is_err());
    assert!(serde_json::from_value::<PermissionRequestCommand>(json!({
        "type":"authorize","request_id":"permission-1",
        "decision":{"type":"deny"},"resolved_at_ms":30
    }))
    .is_ok());
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type": "permission_request",
        "command": {
            "type": "enqueue",
            "input": {
                "request_id": "permission-1", "task_id": "task-1", "child_session_id": "child-1",
                "capability": "write_file", "input": {"path":"/tmp/report.md"},
                "input_json": "{\"path\":\"/tmp/report.md\"}",
                "scope_rule": "write_file:/tmp/report.md", "created_at_ms": 10
            }
        }
    }))
    .is_err());

    let sel_input = |selection| {
        serde_json::from_value::<SelectConversationExecutionInput>(json!({
            "conversation_id":"c", "expected_revision":7, "updated_at_ms":2, "selection":selection,
        }))
        .unwrap()
    };
    let selected = sel_input(json!({
        "kind":"model","model_id":"configured:profile",
        "parameters":{"temperature":0.2}, "presentation":{"showThinking":true}
    }))
    .into_patch()
    .unwrap();
    let patch: Value = serde_json::from_str(&selected.patch_json).unwrap();
    assert_eq!(patch["executionHostBinding"], Value::Null);
    assert_eq!(patch["clientHostParameters"], Value::Null);
    assert_eq!(patch["modelOverrides"]["configured:profile"]["temperature"], 0.2);
    assert_eq!(selected.expected_revision, 7);
    assert!(sel_input(json!({"kind":"model","model_id":"wire-name","parameters":{},"presentation":{}}))
        .into_patch()
        .is_err());
    assert!(sel_input(json!({"kind":"host","binding":{"baseURL":"https://user:secret@example.com"},"parameters":{},"presentation":{}}))
        .into_patch()
        .is_err());
    let inherited = sel_input(json!({"kind":"inherit"})).into_patch().unwrap();
    assert!(inherited.patch_json.fields()["modelSelectionID"].is_null());
    assert!(inherited.patch_json.fields()["executionHostBinding"].is_null());
    let location = sel_input(json!({"kind":"location","execution_host":"remote"}))
        .into_patch()
        .unwrap();
    assert_eq!(location.patch_json.fields().len(), 1);
    assert_eq!(location.patch_json.fields()["executionHostRawValue"], "remote");
    let mut conflicting = sel_input(json!({"kind":"inherit"}));
    conflicting.additional_configuration = Some(
        json!({"modelSelectionID":"configured:other"})
            .to_string()
            .parse()
            .unwrap(),
    );
    assert!(conflicting.into_patch().is_err());
}

#[test]
fn permission_configuration_snapshot_grants_precedence_and_remote_mirror_isolation() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-ledger-suite.db");
    let mirror_conv = "permission-mirror-conversation";
    let snapshot = AgentCore::try_new(AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string(mirror_conv.to_string())),
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
        // 1. Inactive snapshot & revision-checked permission commit ("c" & "stale-c").
        for id in ["c", "stale-c"] {
            store
                .create_conversation(&CreateConversationInput {
                    id: id.into(),
                    title: "Conversation".into(),
                    revision: 1,
                    settings_json: Some("{}".into()),
                    created_at_ms: 1,
                    updated_at_ms: 1,
                })
                .unwrap();
        }
        let read_c = || {
            serde_json::from_value::<ApplicationCommand>(
                json!({"type":"read_permission_configuration","conversation_id":"c"}),
            )
            .unwrap()
        };
        let initial = execute_application_command(&mut store, read_c()).unwrap();
        assert_eq!(initial["owner"], json!({"kind":"inactive"}));
        assert_eq!(initial["state"], "inactive");
        assert_eq!(initial["configuration_revision"], 1);
        let write_c = |revision| {
            serde_json::from_value::<ApplicationCommand>(json!({
                "type":"set_conversation_permission", "conversation_id":"c",
                "expected_revision":revision, "expected_run_id":null, "expected_source_host_id":null,
                "profile":"allow_all", "updated_at_ms":2,
                "additional_configuration":json!({"memoryEnabledOverride":false}).to_string(),
            }))
            .unwrap()
        };
        assert_eq!(
            execute_application_command(&mut store, write_c(1)).unwrap()["status"],
            "committed"
        );
        assert_eq!(
            execute_application_command(&mut store, write_c(1)).unwrap()["status"],
            "conflict"
        );
        let current = execute_application_command(&mut store, read_c()).unwrap();
        assert_eq!(current["configuration_revision"], 2);
        assert_eq!(current["permission_profile"], "allow_all");
        assert_eq!(current["settings"]["memoryEnabledOverride"], false);

        let stale_owner = serde_json::from_value::<ApplicationCommand>(json!({
            "type":"set_conversation_permission", "conversation_id":"stale-c",
            "expected_revision":1, "expected_run_id":"old-run", "expected_source_host_id":null,
            "profile":"allow_all", "updated_at_ms":2,
        }))
        .unwrap();
        assert!(execute_application_command(&mut store, stale_owner)
            .unwrap_err()
            .contains("owner changed"));
        for extra in [
            json!({"unresolved_run_ids":["unknown-run"]}),
            json!({"source_unconfirmed":true}),
        ] {
            let mut read = json!({"type":"read_permission_configuration","conversation_id":"stale-c"});
            read.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                execute_application_command(&mut store, serde_json::from_value(read).unwrap())
                    .is_err()
            );
        }
        assert_eq!(
            store
                .load_conversation("stale-c")
                .unwrap()
                .unwrap()
                .configuration_revision,
            1
        );

        // 2. Typed host grant via ApplicationCommand ("mac-local").
        let recorded: PermissionGrant = serde_json::from_value(
            execute_application_command(
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
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            recorded.target,
            PermissionGrantTarget::Host {
                host_id: "mac-local".into()
            }
        );

        // 3. Durable permission precedence ledger ("permission-conversation").
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
            ("global", PermissionGrantScope::UserGlobal, None, None, StoredPermissionDecision::Deny, 2),
            ("host", PermissionGrantScope::Host, None, Some("mac"), StoredPermissionDecision::Allow, 3),
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
                    } else {
                        "*".into()
                    },
                    decision,
                    created_at_ms: updated_at_ms,
                    updated_at_ms,
                })
                .unwrap();
        }

        // 4. Remote run + retained legacy mirror bytes ("permission-mirror-conversation").
        store
            .admit_remote_fixture(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "mac-source".into(),
                conversation_id: mirror_conv.into(),
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

    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute(
            "INSERT INTO remote_session_mirrors(conversation_id,source_host_id,core_revision,event_cursor,snapshot_json,updated_at_ms,run_id)
             VALUES(?1,'mac-source',0,0,?2,200,'permission-mirror-run')",
            rusqlite::params![mirror_conv, serde_json::to_string(&snapshot).unwrap()],
        )
        .unwrap();
    drop(connection);

    // Cold reopen verification of grant port, precedence rules, and remote mirror non-authority.
    let mut reopened = NolaStore::open(&database).unwrap();
    let listed: Vec<PermissionGrant> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::ListPermissionGrants {
                conversation_id: None,
                host_id: Some("mac-local".into()),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .any(|grant| grant.id == "host-write" && grant.decision == PermissionGrantDecision::Allow));

    let subject = PermissionSubject {
        capability: "write_file".into(),
        resource: r#"{"path":"/workspace/result.txt"}"#.into(),
    };
    let conv_rules =
        load_permission_rule_set(&reopened, Some("permission-conversation"), Some("mac")).unwrap();
    assert_eq!(conv_rules.evaluate(&subject), PermissionPolicyDecision::Deny);
    let host_rules = load_permission_rule_set(&reopened, None, Some("mac")).unwrap();
    assert_eq!(host_rules.evaluate(&subject), PermissionPolicyDecision::Ask);
    assert_eq!(
        host_rules
            .evaluate_resolved_for_origin_with_explanation(
                &subject,
                nola_agent_engine::agent::protocol::AgentInputOrigin::InteractiveUser,
                false
            )
            .decision,
        PermissionPolicyDecision::Allow
    );
    let global_rules = load_permission_rule_set(&reopened, None, Some("linux")).unwrap();
    assert_eq!(global_rules.evaluate(&subject), PermissionPolicyDecision::Deny);

    let mirror_rules = load_permission_rule_set(&reopened, Some(mirror_conv), Some("mac-source"))
        .unwrap();
    assert_ne!(
        mirror_rules.evaluate(&PermissionSubject {
            capability: "write_file".into(),
            resource: r#"{"path":"/workspace/report.md"}"#.into(),
        }),
        PermissionPolicyDecision::Allow
    );
    assert!(nola_application::load_session_projection(&reopened, mirror_conv)
        .unwrap()
        .is_none());
}

fn projection_checkpoint() -> Value {
    json!({
        "revision": 1, "session_id": "permissions", "last_event_cursor": {"sequence": 7},
        "state": {"session_id": "permissions", "execution": {
            "permissions": {"revision": 5, "pending": []}
        }}
    })
}

fn save_projection_for_run(store: &mut NolaStore, payload: Value, run_id: &str) {
    let frontier = admission_frontier::read(store, "permissions");
    let revision = store
        .load_checkpoint("permissions")
        .unwrap()
        .map_or(1, |checkpoint| checkpoint.revision + 1);
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "permissions".into(),
            conversation_title: "Permissions".into(),
            user_message_id: format!("{run_id}:user"),
            user_part_id: format!("{run_id}:user:text"),
            user_text: "fixture".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: format!("{run_id}:assistant"),
            assistant_part_id: format!("{run_id}:assistant:text"),
            run_id: run_id.into(),
            context_checkpoint_id: None,
            created_at_ms: 1,
            checkpoint_revision: revision,
            checkpoint_event_cursor: 7,
            checkpoint_json: payload.to_string(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "permission-fixture".into(),
            outbox_effects: vec![],
            initial_events: vec![nola_store_port::run_creation::InitialCoreRunEvent {
                core_session_id: "permissions".into(),
                core_turn_id: Some(run_id.into()),
                core_cursor: 7,
                core_revision: 1,
                event_kind: "turn_started".into(),
                payload_json:
                    json!({"type":"turn_started","session_id":"permissions","turn_id":run_id})
                        .to_string(),
            }],
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
        })
        .unwrap();
}

fn query_permission_projection(store: &mut NolaStore) -> Result<Value, String> {
    execute_application_command(
        store,
        ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "permissions".into(),
            run_id: "permission-run".into(),
            scope: PermissionProjectionReadScope::CurrentRun,
        },
    )
}

#[test]
fn permission_projection_local_frontier_control_and_corruption_guards() {
    // 1. Run identity, active local owner snapshot & save, selective hot/cold frontier,
    //    10MB unrelated checkpoint exclusion, interruption receipt preservation, and control scope pins.
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "permissions".into(),
            title: "Permissions".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let interruption_receipt = json!({
        "request_id": "request-interrupted", "turn_id": "turn-1", "call_id": "call-1",
        "capability_id": "file.read", "resource": "file:///workspace/document.txt",
        "outcome": {"type": "interrupted", "reason": "user cancelled the Run"}
    });
    let mut big_payload = projection_checkpoint();
    big_payload["state"]["unrelated_history"] = json!("x".repeat(10_000_000));
    big_payload["state"]["execution"]["permissions"]["receipts"] =
        json!([interruption_receipt.clone()]);
    save_projection_for_run(&mut store, big_payload, "permission-run");

    // Run identity query.
    let run_id_cmd: ApplicationCommand = serde_json::from_value(json!({
        "type":"load_run_identity", "run_id":"permission-run"
    }))
    .unwrap();
    assert!(run_id_cmd.is_read_only_query());
    assert_eq!(
        execute_application_command(&mut store, run_id_cmd).unwrap(),
        json!({"run_id":"permission-run", "conversation_id":"permissions"})
    );
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::LoadRunIdentity {
            run_id: "missing".into()
        }
    )
    .unwrap_err()
    .contains("run_not_found"));
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::LoadRunIdentity {
            run_id: " permission-run ".into()
        }
    )
    .is_err());

    // Bounded selective read & interruption receipt exclusion from control projection.
    assert!(
        store
            .load_session_permission_inputs("permissions", None)
            .unwrap()
            .unwrap()
            .len()
            < 1024
    );
    let view = query_permission_projection(&mut store).unwrap();
    assert_eq!(
        view,
        json!({
            "conversation_id":"permissions",
            "run_id":"permission-run",
            "session_id":"permissions",
            "active_turn_id":null,
            "event_cursor":7,
            "permission_projection_revision":5,
            "permissions":{"revision": 5, "pending": [], "active_policy":null}
        })
    );
    assert!(view["permissions"].get("receipts").is_none());
    let saved_ckpt: Value = serde_json::from_str(
        &store
            .load_checkpoint("permissions")
            .unwrap()
            .unwrap()
            .checkpoint_json,
    )
    .unwrap();
    assert_eq!(
        saved_ckpt["state"]["execution"]["permissions"]["receipts"],
        json!([interruption_receipt])
    );
    assert!(store
        .effective_permission_grants(Some("permissions"), None)
        .unwrap()
        .is_empty());

    // Selective hot vs cold CurrentPermissionState frontier.
    let stored_ckpt = store.load_checkpoint("permissions").unwrap().unwrap();
    let current_state = CurrentPermissionState {
        active_permission_policy: None,
        conversation_id: "permissions".into(),
        revision: stored_ckpt.revision,
        event_cursor: 7,
        session_id: "permissions".into(),
        state_session_id: "permissions".into(),
        active_turn_id: None,
        permissions: json!({"revision": 5, "pending": []}),
    };
    let cold: Value = serde_json::from_str(
        &store
            .load_session_permission_inputs("permissions", None)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let hot: Value = serde_json::from_str(
        &store
            .load_session_permission_inputs("permissions", Some(&current_state))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(hot, cold);
    let mut stale_rev = current_state.clone();
    stale_rev.revision += 1;
    assert!(store
        .load_session_permission_inputs("permissions", Some(&stale_rev))
        .is_err());
    let mut foreign_conv = current_state.clone();
    foreign_conv.conversation_id = "foreign".into();
    assert!(store
        .load_session_permission_inputs("permissions", Some(&foreign_conv))
        .is_err());
    let mut bad_cursor = current_state;
    bad_cursor.event_cursor += 1;
    assert!(store
        .load_session_permission_inputs("permissions", Some(&bad_cursor))
        .is_err());

    // Control scope reads bind empty pending to exact owner and revision.
    let control_cmd = json!({
        "type":"load_conversation_permission_projection_v1",
        "conversation_id":"permissions", "run_id":"permission-run",
        "scope":{"type":"control", "session_id":"permissions", "active_turn_id":null,
            "event_cursor":6, "permission_projection_revision":5}
    });
    let current_ctrl = execute_application_command(
        &mut store,
        serde_json::from_value(control_cmd.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(current_ctrl["event_cursor"], 7);
    assert_eq!(current_ctrl["permissions"]["pending"], json!([]));
    for (pointer, val) in [
        ("/run_id", json!("foreign-run")),
        ("/scope/session_id", json!("foreign-session")),
        ("/scope/active_turn_id", json!("foreign-turn")),
        ("/scope/event_cursor", json!(8)),
        ("/scope/permission_projection_revision", json!(4)),
        ("/scope/permission_projection_revision", json!(6)),
    ] {
        let mut invalid = control_cmd.clone();
        *invalid.pointer_mut(pointer).unwrap() = val;
        assert!(
            execute_application_command(&mut store, serde_json::from_value(invalid).unwrap())
                .is_err(),
            "pointer={pointer}"
        );
    }
    let mut missing_turn = control_cmd;
    missing_turn["scope"]
        .as_object_mut()
        .unwrap()
        .remove("active_turn_id");
    assert!(serde_json::from_value::<ApplicationCommand>(missing_turn).is_err());
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type":"load_conversation_permission_projection_v1", "conversation_id":"permissions"
    }))
    .is_err());

    // Active local configuration snapshot & save share the exact active owner.
    let read_cfg: ApplicationCommand = serde_json::from_value(json!({
        "type":"read_permission_configuration", "conversation_id":"permissions",
    }))
    .unwrap();
    assert!(read_cfg.is_read_only_query());
    let snap = execute_application_command(&mut store, read_cfg).unwrap();
    assert_eq!(
        snap["owner"],
        json!({"kind":"local","run_id":"permission-run","assistant_message_id":"permission-run:assistant"})
    );
    assert_eq!(snap["state"], "pending");
    let rev = snap["configuration_revision"].as_i64().unwrap();
    let patch = |run: Option<&str>| {
        serde_json::from_value::<ApplicationCommand>(json!({
            "type":"set_conversation_permission", "conversation_id":"permissions",
            "expected_revision":rev, "expected_run_id":run, "expected_source_host_id":null,
            "profile":"allow_all", "updated_at_ms":2,
        }))
        .unwrap()
    };
    assert!(execute_application_command(&mut store, patch(None))
        .unwrap_err()
        .contains("owner changed"));
    assert!(execute_application_command(&mut store, patch(Some("foreign")))
        .unwrap_err()
        .contains("owner changed"));
    assert_eq!(
        execute_application_command(&mut store, patch(Some("permission-run"))).unwrap()["status"],
        "committed"
    );
    let pending_cfg = execute_application_command(
        &mut store,
        serde_json::from_value(json!({
            "type":"read_permission_configuration", "conversation_id":"permissions",
            "minimum_policy_revision":rev + 1,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(pending_cfg["configuration_revision"], rev + 1);
    assert_eq!(pending_cfg["permission_profile"], "allow_all");
    assert_eq!(pending_cfg["state"], "pending");

    // 2. Corrupt checkpoint identity/cursor and missing turn_started admission evidence on disk.
    let directory = tempdir().unwrap();
    let corrupt_path = directory.path().join("corrupt-permission.db");
    let mut disk_store = NolaStore::open(&corrupt_path).unwrap();
    disk_store
        .create_conversation(&CreateConversationInput {
            id: "permissions".into(),
            title: "Permissions".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    save_projection_for_run(&mut disk_store, projection_checkpoint(), "permission-run");
    let sql = rusqlite::Connection::open(&corrupt_path).unwrap();
    for invalid in 0..5 {
        let mut bad = projection_checkpoint();
        match invalid {
            0 => bad["session_id"] = json!("wrong"),
            1 => bad["state"]["session_id"] = json!("wrong"),
            2 => bad["last_event_cursor"]["sequence"] = json!(8),
            3 => bad["last_event_cursor"]["sequence"] = json!(true),
            _ => {
                bad["session_id"] = json!("same-foreign-session");
                bad["state"]["session_id"] = json!("same-foreign-session");
            }
        }
        sql.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='permissions'",
            [bad.to_string()],
        )
        .unwrap();
        assert!(query_permission_projection(&mut disk_store).is_err());
    }
    sql.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='permissions'",
        [projection_checkpoint().to_string()],
    )
    .unwrap();
    assert!(query_permission_projection(&mut disk_store).unwrap().is_object());
    sql.execute(
        "DELETE FROM run_events WHERE run_id='permission-run' AND event_kind='turn_started'",
        [],
    )
    .unwrap();
    assert!(query_permission_projection(&mut disk_store)
        .unwrap_err()
        .contains("admission evidence"));
}

#[test]
fn permission_projection_remote_ownership_and_successor_run_isolation() {
    // 1. Remote snapshot & stale local checkpoint exclusion when remote run owns conversation.
    let mut remote_store = NolaStore::open(":memory:").unwrap();
    remote_store
        .create_conversation(&CreateConversationInput {
            id: "permissions".into(),
            title: "Permissions".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    assert!(query_permission_projection(&mut remote_store).unwrap().is_null());
    remote_store
        .save_checkpoint(&nola_store::SaveCheckpointInput {
            conversation_id: "permissions".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 7,
            checkpoint_json: projection_checkpoint().to_string(),
            updated_at_ms: 2,
        })
        .unwrap();
    assert!(query_permission_projection(&mut remote_store).is_err());

    remote_store
        .admit_remote_fixture(&AdmittedReplicaTurnInput {
            user_part_kind: "text".into(),
            source_host_id: "host".into(),
            conversation_id: "permissions".into(),
            conversation_title: "Permissions".into(),
            user_message_id: "remote-user".into(),
            user_part_id: "remote-user-body".into(),
            user_text: "Read".into(),
            user_content_json: None,
            assistant_message_id: "remote-assistant".into(),
            assistant_part_id: "remote-body".into(),
            run_id: "remote-run".into(),
            context_checkpoint_id: None,
            created_at_ms: 1,
        })
        .unwrap();
    assert!(query_permission_projection(&mut remote_store).unwrap().is_null());

    let read_remote = |host, client| {
        serde_json::from_value::<ApplicationCommand>(json!({
            "type":"read_permission_configuration", "conversation_id":"permissions",
            "source_host_id":host, "client_id":client,
        }))
        .unwrap()
    };
    assert!(execute_application_command(&mut remote_store, read_remote("foreign", "fixture-client")).is_err());
    assert!(execute_application_command(&mut remote_store, read_remote("host", "foreign-client")).is_err());
    let snap =
        execute_application_command(&mut remote_store, read_remote("host", "fixture-client"))
            .unwrap();
    assert_eq!(snap["owner"]["kind"], "remote");
    assert_eq!(snap["owner"]["identity"]["submission_id"], "remote-run");
    assert!(snap["active_policy"].is_null());
    assert_eq!(snap["state"], "pending");
    let rev = snap["configuration_revision"].as_i64().unwrap();
    let write_remote = serde_json::from_value::<ApplicationCommand>(json!({
        "type":"set_conversation_permission", "conversation_id":"permissions",
        "expected_revision":rev, "expected_run_id":"remote-run",
        "expected_source_host_id":"host", "profile":"allow_all", "updated_at_ms":2,
    }))
    .unwrap();
    assert_eq!(
        execute_application_command(&mut remote_store, write_remote).unwrap()["status"],
        "committed"
    );
    assert!(remote_store.peek_agent_command("permissions").unwrap().is_none());

    // 2. Successor run cannot reuse predecessor permission view.
    let directory = tempdir().unwrap();
    let path = directory.path().join("permission-owner.db");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "permissions".into(),
            title: "Permissions".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    save_projection_for_run(&mut store, projection_checkpoint(), "permission-run");
    assert_eq!(
        query_permission_projection(&mut store).unwrap()["run_id"],
        "permission-run"
    );
    terminal_sources::settle_fixture(
        &mut store,
        "permissions",
        "permission-run",
        "permission-run",
        "permission-run:user",
    );
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE runs SET created_at_ms=999 WHERE id='permission-run'", [])
        .unwrap();
    save_projection_for_run(&mut store, projection_checkpoint(), "successor-run");
    assert!(query_permission_projection(&mut store).is_err());
    let successor = execute_application_command(
        &mut store,
        ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "permissions".into(),
            run_id: "successor-run".into(),
            scope: PermissionProjectionReadScope::CurrentRun,
        },
    )
    .unwrap();
    assert_eq!(successor["run_id"], "successor-run");
    assert_eq!(
        successor["permissions"],
        json!({"revision":5,"pending":[],"active_policy":null})
    );
}

fn pending_request() -> NewPermissionRequest {
    NewPermissionRequest {
        core_permission_request_id: Some("core-fixture-request".into()),
        permission_instructions: Vec::new(),
        request_id: "permission-1".into(),
        task_id: "task-1".into(),
        child_session_id: "child-1".into(),
        capability: "write_file".into(),
        input: json!({"path":"/tmp/report.md","content":"hello"}),
        scope_rule: "write_file:/tmp/report.md".into(),
        created_at_ms: 10,
    }
}

fn parent_task() -> RuntimeTask {
    let mut task = RuntimeTask {
        id: "task-1".into(),
        kind: "agent".into(),
        status: "queued".into(),
        title: "Permission owner".into(),
        project_root: "/workspace".into(),
        owner_session_id: Some("parent-session".into()),
        parent_task_id: None,
        parent_tool_call_id: None,
        child_session_id: Some("child-1".into()),
        agent_type: Some("general".into()),
        prompt: Some("write a report".into()),
        progress_summary: None,
        last_activity: None,
        result_summary: None,
        error: None,
        output_path: None,
        unread: false,
        cancel_requested: false,
        cancel_requested_at_ms: None,
        worker_id: None,
        ownership_token: None,
        lease_until_ms: None,
        heartbeat_at_ms: None,
        retry_count: 0,
        retry_at_ms: None,
        last_error: None,
        input_watermark: None,
        completion_watermark: None,
        created_at_ms: 1,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 1,
        metadata_json: "{}".into(),
    };
    task.set_scheduling(RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(60_000),
            max_output_bytes: Some(1_048_576),
            max_cpu_time_ms: None,
            max_memory_bytes: None,
            max_processes: None,
        },
        output_bytes: 0,
        resource_usage: RuntimeTaskResourceUsage::default(),
        budget_exhaustion: None,
    })
    .unwrap();
    task
}

fn setup_scope_resolution(store: &mut NolaStore) {
    for task_id in ["task-1", "task-other"] {
        let mut task = parent_task();
        task.id = task_id.into();
        execute_application_command(
            store,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(task).unwrap(),
                },
            },
        )
        .unwrap();
    }
    for id in [
        "permission-1",
        "same-scope",
        "other-resource",
        "other-capability",
        "other-task",
        "other-child",
        "other-scope",
        "with-guidance",
    ] {
        let mut input = pending_request();
        input.request_id = id.into();
        input.core_permission_request_id = Some(format!("core-{id}"));
        input.scope_rule =
            json!({"version":2,"permission":"write","resource":"/tmp/report.md","tool":"write_file"})
                .to_string();
        match id {
            "other-resource" => input.input = json!({"path":"/tmp/different.md"}),
            "other-capability" => input.capability = "read_file".into(),
            "other-task" => input.task_id = "task-other".into(),
            "other-child" => input.child_session_id = "child-other".into(),
            "other-scope" => input.scope_rule = "different-scope".into(),
            "with-guidance" => input.permission_instructions.push(PermissionInstruction {
                permission_request_id: "earlier-core-wait".into(),
                instruction: "Preserve comments".into(),
            }),
            _ => {}
        }
        execute_application_command(
            store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue { input },
            },
        )
        .unwrap();
    }
}

fn scope_decision(request_id: &str, allow: bool, at: i64) -> ApplicationCommand {
    ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Authorize {
            request_id: request_id.into(),
            decision: if allow {
                PermissionAuthorizationDecision::AllowConversation
            } else {
                PermissionAuthorizationDecision::Deny
            },
            resolved_at_ms: at,
        },
    }
}

#[test]
fn scope_resolution_atomic_grant_rollback_sibling_isolation_and_transport_replay() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("scope-resolution.db");
    let mut store = NolaStore::open(&database).unwrap();
    setup_scope_resolution(&mut store);

    // 1. First scope resolution cannot commit without its matching atomic grant.
    assert!(store
        .resolve_permission_request("permission-1", "allow_conversation", None, 100, None)
        .is_err());
    let req = store.permission_request("permission-1").unwrap().unwrap();
    let mismatched_grant = UpsertPermissionGrantInput {
        id: "permission-request:permission-1:task-scope".into(),
        scope: PermissionGrantScope::Host,
        conversation_id: None,
        host_id: Some("task:task-1".into()),
        capability_pattern: "cli::tool::allow_rule".into(),
        resource_pattern: req.scope_rule,
        decision: StoredPermissionDecision::Allow,
        created_at_ms: 99,
        updated_at_ms: 100,
    };
    assert!(store
        .resolve_permission_request(
            "permission-1",
            "allow_conversation",
            None,
            100,
            Some(&mismatched_grant)
        )
        .is_err());
    assert_eq!(
        store.permission_request("permission-1").unwrap().unwrap().status,
        "pending"
    );
    assert!(store
        .effective_permission_grants(None, Some("task:task-1"))
        .unwrap()
        .is_empty());

    // 2. Scope resolution write failure rolls back original, siblings, and derived grant.
    let inspection = rusqlite::Connection::open(&database).unwrap();
    inspection
        .execute_batch(
            "CREATE TRIGGER reject_scope_original BEFORE UPDATE ON permission_requests
             WHEN NEW.request_id='permission-1' AND NEW.status='resolved'
             BEGIN SELECT RAISE(ABORT,'scope-write-fault'); END;",
        )
        .unwrap();
    assert!(execute_application_command(&mut store, scope_decision("permission-1", true, 100)).is_err());
    for id in ["permission-1", "same-scope"] {
        let r = store.permission_request(id).unwrap().unwrap();
        assert_eq!(r.status, "pending");
        assert!(r.decision.is_none() && r.resolved_at_ms.is_none());
    }
    assert!(store
        .effective_permission_grants(None, Some("task:task-1"))
        .unwrap()
        .is_empty());
    inspection
        .execute_batch("DROP TRIGGER reject_scope_original;")
        .unwrap();
    drop(store);

    // 3. Receipt transport replay vs new transport ID vs conflicting transport ID, sibling isolation, and grant revocation.
    let mut runtime = support::open_runtime(&database).unwrap();
    let submit = |runtime: &mut support::ApplicationRuntime, transport_id: &str, allow: bool, at: i64| {
        let command = scope_decision("permission-1", allow, at);
        let envelope = ExecuteReceivedCommand {
            client_id: "permission-owner".into(),
            request_id: transport_id.into(),
            command_type: "permission_request".into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None,
            received_at: "2026-10-12T00:00:00Z".into(),
        };
        match runtime
            .execute_received_command(envelope, command)
            .unwrap()
            .outcome
        {
            ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
            other => panic!("resolution command was not committed: {other:?}"),
        }
    };
    let first = submit(&mut runtime, "first-transport", true, 100);
    assert_eq!(first["type"], "resolved");
    assert_eq!(first["receipt"]["request_id"], "permission-1");
    assert_eq!(first["receipt"]["task_id"], "task-1");
    assert_eq!(first["receipt"]["child_session_id"], "child-1");
    assert_eq!(first["receipt"]["core_permission_request_id"], "core-permission-1");
    assert_eq!(first["receipt"]["resolved_at_ms"], 100);
    assert!(first["receipt"].get("input").is_none());
    assert_eq!(submit(&mut runtime, "first-transport", true, 100), first);
    let replay_new_id = submit(&mut runtime, "new-transport", true, 200);
    assert_eq!(replay_new_id["type"], "already_resolved");
    assert_eq!(replay_new_id["receipt"], first["receipt"]);
    let conflict_id = submit(&mut runtime, "conflicting-transport", false, 300);
    assert_eq!(conflict_id["type"], "conflict");
    assert_eq!(conflict_id["receipt"], first["receipt"]);
    drop(runtime);

    let mut store = NolaStore::open(&database).unwrap();
    for id in [
        "same-scope",
        "other-resource",
        "other-capability",
        "other-task",
        "other-child",
        "other-scope",
        "with-guidance",
    ] {
        let sibling = store.permission_request(id).unwrap().unwrap();
        assert_eq!(sibling.status, "pending");
        assert!(sibling.decision.is_none());
    }
    let grants = store
        .effective_permission_grants(None, Some("task:task-1"))
        .unwrap();
    assert_eq!(grants.len(), 1);
    store.revoke_permission_grant(&grants[0].id, 101).unwrap();
    let mut late = pending_request();
    late.request_id = "late-wait".into();
    late.core_permission_request_id = Some("late-core".into());
    late.scope_rule = grants[0].resource_pattern.clone();
    execute_application_command(
        &mut store,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue { input: late },
        },
    )
    .unwrap();
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    let replay = execute_application_command(&mut store, scope_decision("permission-1", true, 999)).unwrap();
    assert_eq!(replay["type"], "already_resolved");
    assert_eq!(replay["receipt"], first["receipt"]);
    let conflict =
        execute_application_command(&mut store, scope_decision("permission-1", false, 1000))
            .unwrap();
    assert_eq!(conflict["type"], "conflict");
    assert!(store
        .effective_permission_grants(None, Some("task:task-1"))
        .unwrap()
        .is_empty());
    assert_eq!(
        store.permission_request("same-scope").unwrap().unwrap().status,
        "pending"
    );
    assert_eq!(
        store.permission_request("late-wait").unwrap().unwrap().status,
        "pending"
    );
    assert_eq!(
        execute_application_command(&mut store, scope_decision("missing", true, 1001)).unwrap(),
        json!({"type":"missing","request_id":"missing"})
    );
}

#[test]
fn permission_interventions_receipt_rollback_and_custom_budget_validation() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-interventions.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
                },
            },
        )
        .unwrap();
        for (req_id, core_id) in [
            ("permission-1", "core-1"),
            ("permission-return", "core-return"),
        ] {
            let mut req = pending_request();
            req.request_id = req_id.into();
            req.core_permission_request_id = Some(core_id.into());
            execute_application_command(
                &mut store,
                ApplicationCommand::PermissionRequest {
                    command: PermissionRequestCommand::Enqueue { input: req },
                },
            )
            .unwrap();
        }
        execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Intervene {
                    request_id: "permission-return".into(),
                    intervention: PermissionIntervention::ReturnToComposer,
                    resolved_at_ms: 15,
                },
            },
        )
        .unwrap();
    }

    // Receipt rollback on ProvideInstruction followed by commit and cold replay.
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect
        .execute_batch(
            "CREATE TRIGGER fail_permission_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed'
             BEGIN SELECT RAISE(ABORT,'permission receipt fault'); END;",
        )
        .unwrap();
    let command = ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Intervene {
            request_id: "permission-1".into(),
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "write it under /tmp/approved instead".into(),
            },
            resolved_at_ms: 20,
        },
    };
    let envelope = ExecuteReceivedCommand {
        client_id: "permission-test".into(),
        request_id: "resolve".into(),
        command_type: "permission_request".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    assert!(runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap()
        )
        .is_err());
    {
        let store = NolaStore::open(&database).unwrap();
        let pending = store.permission_request("permission-1").unwrap().unwrap();
        assert_eq!(pending.status, "pending");
        assert!(pending.decision.is_none() && pending.resolved_at_ms.is_none());
    }
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    inspect
        .execute_batch("DROP TRIGGER fail_permission_receipt")
        .unwrap();

    let committed = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);

    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime.execute_received_command(envelope, command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    drop(runtime);

    let mut reopened = NolaStore::open(&database).unwrap();
    let resolved_one: Option<PermissionRequest> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Get {
                    request_id: "permission-1".into(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolved_one.unwrap().resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "write it under /tmp/approved instead".into(),
            },
        })
    );
    let resolved_return: Option<PermissionRequest> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Get {
                    request_id: "permission-return".into(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let resolved_return = resolved_return.unwrap();
    assert_eq!(resolved_return.status, PermissionRequestStatus::Resolved);
    assert_eq!(
        resolved_return.resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ReturnToComposer,
        })
    );

    // Custom instruction budget & legacy explicit-decision checks on the same store.
    for case in 0..4 {
        let req_id = format!("budget-{case}");
        let mut input = pending_request();
        input.request_id = req_id.clone();
        input.core_permission_request_id = Some(format!("core-budget-{case}"));
        let instruction = match case {
            0 => "x".repeat(4097),
            1 => {
                input.permission_instructions = (0..16)
                    .map(|i| PermissionInstruction {
                        permission_request_id: format!("core-{i}"),
                        instruction: "x".into(),
                    })
                    .collect();
                "next".into()
            }
            2 => {
                input.permission_instructions = (0..3)
                    .map(|i| PermissionInstruction {
                        permission_request_id: format!("core-{i}"),
                        instruction: "x".repeat(4096),
                    })
                    .collect();
                "x".repeat(4096)
            }
            _ => {
                input.core_permission_request_id = None;
                "legacy instruction".into()
            }
        };
        execute_application_command(
            &mut reopened,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue { input },
            },
        )
        .unwrap();
        let before = reopened.permission_request(&req_id).unwrap().unwrap();
        assert!(execute_application_command(
            &mut reopened,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Intervene {
                    request_id: req_id.clone(),
                    intervention: PermissionIntervention::ProvideInstruction {
                        instruction: instruction.clone()
                    },
                    resolved_at_ms: 20,
                },
            }
        )
        .is_err());
        assert!(reopened
            .resolve_permission_request(&req_id, "provide_instruction", Some(&instruction), 20, None)
            .is_err());
        let after = reopened.permission_request(&req_id).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert!(matches!(
            reopened
                .resolve_permission_request(&req_id, "allow_once", None, 21, None)
                .unwrap(),
            StoredPermissionResolutionOutcome::Resolved(_)
        ));
    }
}

fn assert_late_permission_admission_rejected(store: &mut NolaStore) {
    for ensure in [false, true] {
        let mut input = pending_request();
        input.request_id = "late-request".into();
        input.core_permission_request_id = Some("late-core-occurrence".into());
        input.created_at_ms = 50;
        let command = if ensure {
            PermissionRequestCommand::EnsureCoreRequest { input }
        } else {
            PermissionRequestCommand::Enqueue { input }
        };
        assert!(
            execute_application_command(store, ApplicationCommand::PermissionRequest { command })
                .is_err()
        );
        assert!(store.permission_request("late-request").unwrap().is_none());
        assert!(store
            .effective_permission_grants(None, Some("task:task-1"))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn unbound_permissions_close_only_with_exact_worker_terminal_receipt() {
    for terminal in ["completed", "failed", "cancelled"] {
        for approved in [false, true] {
            let directory = tempdir().unwrap();
            let database = directory.path().join("unbound-running-stop.db");
            let mut store = NolaStore::open(&database).unwrap();
            let mut task = parent_task();
            task.kind = "shell_process".into();
            let watermark = task.input_watermark;
            execute_application_command(
                &mut store,
                ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::Create {
                        input: CreateRuntimeTask::try_from(task).unwrap(),
                    },
                },
            )
            .unwrap();
            let attempt = store
                .claim_runtime_task("task-1", "worker", "generation", "token", 20, 1000)
                .unwrap()
                .unwrap();
            execute_application_command(
                &mut store,
                ApplicationCommand::PermissionRequest {
                    command: PermissionRequestCommand::Enqueue {
                        input: pending_request(),
                    },
                },
            )
            .unwrap();
            if approved {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::Authorize {
                            request_id: "permission-1".into(),
                            decision: PermissionAuthorizationDecision::AllowConversation,
                            resolved_at_ms: 25,
                        },
                    },
                )
                .unwrap();
                assert_eq!(
                    store
                        .effective_permission_grants(None, Some("task:task-1"))
                        .unwrap()
                        .len(),
                    1
                );
            }
            let before = store.permission_request("permission-1").unwrap().unwrap();
            if terminal == "cancelled" {
                assert!(store.request_runtime_task_cancel("task-1", 30).unwrap());
                assert!(store
                    .effective_permission_grants(None, Some("task:task-1"))
                    .unwrap()
                    .is_empty());
                assert_late_permission_admission_rejected(&mut store);
            }
            assert_eq!(store.runtime_task("task-1").unwrap().unwrap().status, "running");
            assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), before);
            assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
            let inspect = rusqlite::Connection::open(&database).unwrap();
            inspect
                .execute_batch(
                    "CREATE TRIGGER reject_terminal_closure BEFORE UPDATE ON permission_requests
                     WHEN NEW.closure_json IS NOT NULL BEGIN SELECT RAISE(ABORT,'closure fault'); END",
                )
                .unwrap();
            assert!(store
                .finish_runtime_task_detailed_with_event(
                    &RuntimeTaskLeaseWitness::from(&attempt),
                    terminal,
                    watermark,
                    None,
                    None,
                    None,
                    None,
                    true,
                    "finished",
                    terminal,
                    None,
                    "{}",
                    None,
                    40,
                )
                .is_err());
            assert_eq!(store.runtime_task("task-1").unwrap().unwrap().status, "running");
            assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
            assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), before);
            assert_eq!(
                store
                    .effective_permission_grants(None, Some("task:task-1"))
                    .unwrap()
                    .len(),
                usize::from(approved && terminal != "cancelled")
            );
            inspect
                .execute_batch("DROP TRIGGER reject_terminal_closure")
                .unwrap();
            assert!(store
                .finish_runtime_task_detailed_with_event(
                    &RuntimeTaskLeaseWitness::from(&attempt),
                    terminal,
                    watermark,
                    None,
                    None,
                    None,
                    None,
                    true,
                    "finished",
                    terminal,
                    None,
                    "{}",
                    None,
                    40,
                )
                .unwrap());
            drop(store);
            let mut store = NolaStore::open(&database).unwrap();
            let closed = store.permission_request("permission-1").unwrap().unwrap();
            let receipt = store.runtime_task_terminal_receipt("task-1").unwrap().unwrap();
            assert_eq!(receipt.status, terminal);
            assert_eq!(closed.status, "withdrawn");
            assert_eq!(closed.decision, before.decision);
            assert_eq!(closed.resolved_at_ms, before.resolved_at_ms);
            assert!(matches!(
                &closed.closure.as_ref().unwrap().source,
                nola_domain::permission_closure::PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id }
                    if terminal_receipt_id == &receipt.receipt_id
            ));
            assert!(matches!(receipt.source, RuntimeTaskTerminalSource::WorkerFinish { .. }));
            assert!(store
                .pending_permissions_for_owner("parent-session")
                .unwrap()
                .is_empty());
            assert_late_permission_admission_rejected(&mut store);
            let replay: PermissionRequest = serde_json::from_value(
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::EnsureCoreRequest {
                            input: pending_request(),
                        },
                    },
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(replay.status, PermissionRequestStatus::Withdrawn);
            assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), closed);
            let mut conflicting = pending_request();
            conflicting.input = json!({"path":"/tmp/different"});
            assert!(execute_application_command(
                &mut store,
                ApplicationCommand::PermissionRequest {
                    command: PermissionRequestCommand::EnsureCoreRequest { input: conflicting },
                }
            )
            .is_err());
            let result = execute_application_command(
                &mut store,
                ApplicationCommand::PermissionRequest {
                    command: PermissionRequestCommand::Authorize {
                        request_id: "permission-1".into(),
                        decision: PermissionAuthorizationDecision::AllowConversation,
                        resolved_at_ms: 50,
                    },
                },
            )
            .unwrap();
            assert!(matches!(
                serde_json::from_value::<PermissionResolutionOutcome>(result).unwrap(),
                PermissionResolutionOutcome::Closed { .. }
            ));
            assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), closed);
        }
    }
}

#[test]
fn committed_stop_fences_decisions_and_revokes_only_derived_grants_atomically() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("stop-and-revoke.db");
    let mut store = NolaStore::open(&database).unwrap();
    let forged_grant = UpsertPermissionGrantInput {
        id: "permission-request:permission-1:task-scope".into(),
        scope: PermissionGrantScope::Host,
        conversation_id: None,
        host_id: Some("task:task-1".into()),
        capability_pattern: "cli::tool::allow_rule".into(),
        resource_pattern: "write_file:/tmp/report.md".into(),
        decision: StoredPermissionDecision::Allow,
        created_at_ms: 15,
        updated_at_ms: 15,
    };
    assert!(store
        .upsert_permission_grant(&forged_grant)
        .unwrap_err()
        .to_string()
        .contains("permission decision"));

    execute_application_command(
        &mut store,
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
            },
        },
    )
    .unwrap();
    let mut approved_req = pending_request();
    approved_req.request_id = "permission-1".into();
    let mut pending_req = pending_request();
    pending_req.request_id = "permission-pending".into();
    pending_req.core_permission_request_id = Some("core-pending".into());
    for req in [approved_req, pending_req] {
        execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue { input: req },
            },
        )
        .unwrap();
    }
    execute_application_command(
        &mut store,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Authorize {
                request_id: "permission-1".into(),
                decision: PermissionAuthorizationDecision::AllowConversation,
                resolved_at_ms: 15,
            },
        },
    )
    .unwrap();
    let original_approved = store.permission_request("permission-1").unwrap();
    let original_pending = store.permission_request("permission-pending").unwrap();

    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute(
        "INSERT INTO runtime_task_run_bindings
         (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
         VALUES('task-1','parent-session','child-1','run','attempt',NULL,10)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO permission_grants
         SELECT 'independent',scope,conversation_id,host_id,capability_pattern,resource_pattern,
         decision,created_at_ms,updated_at_ms,revoked_at_ms FROM permission_grants
         WHERE id='permission-request:permission-1:task-scope'",
        [],
    )
    .unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_revoke BEFORE UPDATE ON permission_grants
         BEGIN SELECT RAISE(ABORT,'fixture revoke failure'); END;",
    )
    .unwrap();
    let original_task = store.runtime_task("task-1").unwrap();
    assert!(store.request_runtime_task_cancel("task-1", 20).is_err());
    assert_eq!(store.runtime_task("task-1").unwrap(), original_task);
    assert_eq!(
        store
            .effective_permission_grants(None, Some("task:task-1"))
            .unwrap()
            .len(),
        2
    );
    db.execute_batch("DROP TRIGGER reject_revoke;").unwrap();
    assert!(store.request_runtime_task_cancel("task-1", 20).unwrap());
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(store.permission_request("permission-1").unwrap(), original_approved);
    assert!(store
        .upsert_permission_grant(&forged_grant)
        .unwrap_err()
        .to_string()
        .contains("permission decision"));
    let grants = store
        .effective_permission_grants(None, Some("task:task-1"))
        .unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, "independent");
    assert!(store.runtime_task("task-1").unwrap().unwrap().cancel_requested);
    assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());

    let mut commands = [
        PermissionAuthorizationDecision::AllowOnce,
        PermissionAuthorizationDecision::AllowConversation,
        PermissionAuthorizationDecision::Deny,
    ]
    .into_iter()
    .map(|decision| PermissionRequestCommand::Authorize {
        request_id: "permission-pending".into(),
        decision,
        resolved_at_ms: 30,
    })
    .collect::<Vec<_>>();
    commands.push(PermissionRequestCommand::Intervene {
        request_id: "permission-pending".into(),
        intervention: PermissionIntervention::ProvideInstruction {
            instruction: "changed scope".into(),
        },
        resolved_at_ms: 30,
    });
    for command in commands {
        let result =
            execute_application_command(&mut store, ApplicationCommand::PermissionRequest { command })
                .unwrap();
        assert_eq!(
            result,
            json!({
                "type":"stop_pending",
                "request_id":"permission-pending",
                "task_id":"task-1",
                "requested_at_ms":20
            })
        );
        assert_eq!(
            store.permission_request("permission-pending").unwrap(),
            original_pending
        );
    }
}

#[test]
fn concurrent_stop_arbitration_and_atomic_ensure_core_permission_recovery() {
    // 1. Concurrent stop and scope authorization serialize to one consistent outcome.
    for iteration in 0..8 {
        let directory = tempdir().unwrap();
        let database = directory.path().join("permission-race.db");
        let mut store = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
                },
            },
        )
        .unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue {
                    input: pending_request(),
                },
            },
        )
        .unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute(
            "INSERT INTO runtime_task_run_bindings
             (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
             VALUES('task-1','parent-session','child-1','run','attempt',NULL,10)",
            [],
        )
        .unwrap();
        drop(db);
        drop(store);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2)
            .map(|side| {
                let path = database.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut store = NolaStore::open(path).unwrap();
                    barrier.wait();
                    if side == 0 {
                        execute_application_command(
                            &mut store,
                            ApplicationCommand::RuntimeTask {
                                command: RuntimeTaskCommand::RequestCancel {
                                    task_id: "task-1".into(),
                                    now_ms: 20,
                                },
                            },
                        )
                    } else {
                        execute_application_command(
                            &mut store,
                            ApplicationCommand::PermissionRequest {
                                command: PermissionRequestCommand::Authorize {
                                    request_id: "permission-1".into(),
                                    decision: PermissionAuthorizationDecision::AllowConversation,
                                    resolved_at_ms: 20,
                                },
                            },
                        )
                    }
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|w| w.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results[0], json!(true), "iteration {iteration}");
        let store = NolaStore::open(&database).unwrap();
        let request = store.permission_request("permission-1").unwrap().unwrap();
        let grants = store
            .effective_permission_grants(None, Some("task:task-1"))
            .unwrap();
        match results[1]["type"].as_str().unwrap() {
            "resolved" => {
                assert_eq!(request.decision.as_deref(), Some("allow_conversation"));
                assert!(grants.is_empty());
                let db = rusqlite::Connection::open(&database).unwrap();
                let revoked: Option<i64> = db
                    .query_row(
                        "SELECT revoked_at_ms FROM permission_grants
                         WHERE id='permission-request:permission-1:task-scope'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(revoked, Some(20));
            }
            "stop_pending" => {
                assert_eq!(request.status, "pending");
                assert!(request.decision.is_none() && grants.is_empty());
                assert_eq!(results[1]["requested_at_ms"], 20);
            }
            other => panic!("unexpected arbitration outcome: {other}"),
        }
    }

    // 2. EnsureCoreRequest receipt atomicity and concurrent delivery recovery.
    let directory = tempdir().unwrap();
    let database = directory.path().join("ensure-permission.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
            },
        },
    )
    .unwrap();
    drop(store);
    let input = pending_request();
    let command = ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::EnsureCoreRequest {
            input: input.clone(),
        },
    };
    let envelope = ExecuteReceivedCommand {
        client_id: "ensure-test".into(),
        request_id: "ensure-first".into(),
        command_type: "permission_request".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect
        .execute_batch(
            "CREATE TRIGGER fail_ensure_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed'
             BEGIN SELECT RAISE(ABORT,'ensure fault'); END",
        )
        .unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap()
        )
        .is_err());
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM permission_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    inspect
        .execute_batch("DROP TRIGGER fail_ensure_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    drop(runtime);

    let pending_attempts: Vec<_> = (0..2)
        .map(|index| {
            let path = database.clone();
            let mut next = input.clone();
            next.request_id = format!("pending-attempt-{index}");
            std::thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::EnsureCoreRequest { input: next },
                    },
                )
                .unwrap()
            })
        })
        .collect();
    for handle in pending_attempts {
        let pending = handle.join().unwrap();
        assert_eq!(pending["request_id"], input.request_id);
        assert_eq!(pending["status"], "pending");
    }
    let mut store = NolaStore::open(&database).unwrap();
    assert!(matches!(
        store
            .resolve_permission_request(
                &input.request_id,
                "provide_instruction",
                Some("Keep formatting"),
                2000,
                None
            )
            .unwrap(),
        StoredPermissionResolutionOutcome::Resolved(_)
    ));
    drop(store);

    let handles: Vec<_> = (0..2)
        .map(|index| {
            let path = database.clone();
            let mut next = input.clone();
            next.request_id = format!("attempt-{index}");
            next.created_at_ms += 100;
            std::thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::EnsureCoreRequest { input: next },
                    },
                )
                .unwrap()
            })
        })
        .collect();
    for handle in handles {
        let result = handle.join().unwrap();
        assert_eq!(result["request_id"], input.request_id);
        assert_eq!(result["status"], "resolved");
        assert_eq!(
            result["resolution"]["intervention"]["instruction"],
            "Keep formatting"
        );
    }
    for field in 0..5 {
        let mut changed = input.clone();
        changed.request_id = format!("conflict-{field}");
        match field {
            0 => changed.child_session_id = "other".into(),
            1 => changed.capability = "other".into(),
            2 => changed.scope_rule = "other".into(),
            3 => changed.input = json!({"changed":true}),
            _ => changed
                .permission_instructions
                .push(PermissionInstruction {
                    permission_request_id: "previous".into(),
                    instruction: "different".into(),
                }),
        }
        let mut store = NolaStore::open(&database).unwrap();
        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::EnsureCoreRequest { input: changed }
            }
        )
        .is_err());
    }
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM permission_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
