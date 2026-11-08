// SPDX-License-Identifier: Apache-2.0
//! Real Application permission translation against a non-SQLite repository.
use nola_application::permission::execute_permission_request_command;
use nola_application::*;
use nola_agent_engine::agent::permissions::{PermissionPolicyDecision, PermissionSubject};
use nola_store_port::permission::*;
use serde_json::json;

#[derive(Default)]
struct Facts {
    request: Option<StoredPermissionRequest>,
    grants: Vec<StoredPermissionGrant>,
    fail: bool,
}
impl PermissionRepository for Facts {
    type Error = &'static str;
    fn ensure_core_permission_request(
        &mut self,
        _: &StoredPermissionRequest,
    ) -> Result<StoredPermissionRequest, Self::Error> {
        Err("fixture operation unavailable".into())
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
            return Ok(StoredPermissionResolutionOutcome::Missing { request_id: request_id.into() });
        };
        if r.status != "pending" {
            return Ok(if r.decision.as_deref() == Some(decision) && r.custom_response.as_deref() == response {
                StoredPermissionResolutionOutcome::AlreadyResolved(r.clone())
            } else { StoredPermissionResolutionOutcome::Conflict(r.clone()) });
        }
        assert!(grant.is_none(), "this fixture exercises instruction resolution only");
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
fn non_sqlite_intervention_preserves_typed_input_and_propagates_failure() {
    let mut facts = Facts::default();
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
    let command = PermissionRequestCommand::Intervene {
        request_id: "r".into(),
        intervention: PermissionIntervention::ProvideInstruction {
            instruction: "  请只修改注释  ".into(),
        },
        resolved_at_ms: 2,
    };
    facts.fail = true;
    assert!(
        execute_permission_request_command(&mut facts, command.clone())
            .unwrap_err()
            .contains("repository unavailable")
    );
    assert_eq!(facts.request.as_ref().unwrap().status, "pending");
    facts.fail = false;
    assert_eq!(
        execute_permission_request_command(&mut facts, command.clone()).unwrap()["type"],
        json!("resolved")
    );
    assert_eq!(
        execute_permission_request_command(&mut facts, command).unwrap()["type"],
        json!("already_resolved")
    );
    let value = execute_permission_request_command(
        &mut facts,
        PermissionRequestCommand::Get {
            request_id: "r".into(),
        },
    )
    .unwrap();
    let request: PermissionRequest = serde_json::from_value(value).unwrap();
    assert_eq!(request.input, json!({"path":"结果.txt"}));
    assert_eq!(
        request.resolution,
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
}

#[test]
fn non_sqlite_ordered_grants_are_evaluated_by_real_core() {
    let mut facts = Facts::default();
    for (scope, decision) in [
        (
            PermissionGrantScope::UserGlobal,
            StoredPermissionDecision::Deny,
        ),
        (PermissionGrantScope::Host, StoredPermissionDecision::Allow),
        (
            PermissionGrantScope::Conversation,
            StoredPermissionDecision::Deny,
        ),
    ] {
        facts.grants.push(StoredPermissionGrant {
            id: format!("{scope:?}"),
            scope,
            conversation_id: None,
            host_id: None,
            capability_pattern: "write_file".into(),
            resource_pattern: if scope == PermissionGrantScope::Conversation {"结果.txt".into()} else {"*".into()},
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
        load_permission_rule_set(&facts,None,None).unwrap().evaluate_resolved_for_origin_with_explanation(&subject,
            nola_agent_engine::agent::protocol::AgentInputOrigin::InteractiveUser,false).decision,
        PermissionPolicyDecision::Allow
    );
    facts.fail = true;
    assert!(load_permission_rule_set(&facts, None, None)
        .unwrap_err()
        .contains("repository unavailable"));
}
