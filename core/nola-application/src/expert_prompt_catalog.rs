// SPDX-License-Identifier: Apache-2.0
//! Read-only expert template choices from the current catalog and resource policy.
//! Hosts supply their authenticated principal; a choice grants no execution.
use nola_agent_engine::agent::resource_access::AgentResourcePrincipal;
use nola_store_port::product_setting::ProductSettingRepository;
use serde_json::{json, Value};

pub fn read<R: ProductSettingRepository>(
    store: &R,
    actor: &AgentResourcePrincipal,
    now_epoch_ms: i64,
    request_host_id: Option<String>,
) -> Result<Value, String> {
    if now_epoch_ms < 0 {
        return Err("invalid_prompt_catalog_time".into());
    }
    let setting = store
        .product_setting("prompt.templates")
        .map_err(|error| error.to_string())?;
    match setting {
        None => Ok(json!({"revision":null,"templates":[]})),
        Some(setting) => project(
            setting.revision,
            &setting.value_json,
            actor,
            now_epoch_ms,
            request_host_id,
        ),
    }
}

fn project(
    revision: i64,
    value: &str,
    actor: &nola_agent_engine::agent::resource_access::AgentResourcePrincipal,
    now_epoch_ms: i64,
    request_host_id: Option<String>,
) -> Result<Value, String> {
    let host = crate::expert_template_capture::HostExpertTemplateCaptureContext {
        actor: actor.clone(),
        request_host_id,
        selections: vec![],
        builtins: vec![],
    };
    host.validate()?;
    let catalog = crate::expert_template_capture::parse_custom_catalog(revision, value)?;
    let mut result = Vec::with_capacity(catalog.templates.len());
    for template in catalog.templates {
        if template.resource.is_none() {
            continue;
        }
        if template.decision(&host, now_epoch_ms)?.allowed {
            result.push(json!({"id":template.id,"name":template.name}));
        }
    }
    Ok(json!({"revision":revision,"templates":result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_store_query_roundtrips_current_contract_and_preserves_invalid_user_catalog() {
        use crate::{execute_application_command, ApplicationCommand};
        let mut store = nola_store::NolaStore::open(":memory:").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let template = json!({"id":id,"name":"Owned", "content":"Definition", "isBuiltIn":false, "description":"Current", "createdAt":0, "updatedAt":0,
            "accessOwnerID":"owner", "accessRevision":1,"accessGrants":[]});
        let snapshot =
            json!({"customTemplates":[template.clone()],"selectedTemplateID":null}).to_string();
        let saved = store
            .put_product_setting("prompt.templates", &snapshot, 1)
            .unwrap();
        let query = |principal: &str| ApplicationCommand::ListReadableExpertTemplates {
            actor: AgentResourcePrincipal {
                kind:
                    nola_agent_engine::agent::resource_access::AgentResourcePrincipalKind::LocalUser,
                id: principal.into(),
            },
            now_epoch_ms: 2,
            request_host_id: None,
        };
        let wire = serde_json::to_value(query("owner")).unwrap();
        assert!(
            serde_json::from_value::<ApplicationCommand>(wire).is_err(),
            "Only the private authenticated Host may inject an actor"
        );
        assert!(query("owner").is_read_only_query());
        assert_eq!(
            execute_application_command(&mut store, query("owner")).unwrap(),
            json!({"revision":saved.revision,"templates":[{"id":id,"name":"Owned"}]})
        );
        assert_eq!(
            execute_application_command(&mut store, query("another-installation")).unwrap()
                ["templates"],
            json!([])
        );
        assert_eq!(
            store
                .product_setting("prompt.templates")
                .unwrap()
                .unwrap()
                .value_json,
            snapshot
        );
        let mut same_uuid = template.clone();
        same_uuid["id"] = json!(id.to_uppercase());
        let malformed = json!({"customTemplates":[template,same_uuid]}).to_string();
        let rejected = store
            .put_product_setting("prompt.templates", &malformed, 3)
            .unwrap();
        assert!(execute_application_command(&mut store, query("owner")).is_err());
        let unchanged = store.product_setting("prompt.templates").unwrap().unwrap();
        assert_eq!(unchanged.value_json, malformed);
        assert_eq!(unchanged.revision, rejected.revision);
    }
    fn project_for_host(revision: i64, value: &str) -> Result<Value, String> {
        project(
            revision,
            value,
            &AgentResourcePrincipal {
                kind:
                    nola_agent_engine::agent::resource_access::AgentResourcePrincipalKind::LocalUser,
                id: "cli-local-user".into(),
            },
            1,
            None,
        )
    }
    #[test]
    fn prompt_picker_reads_current_custom_definitions_without_exposing_bodies_or_inventing_builtins(
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        let template = json!({"id":id,"name":"Expert rules","content":"private definition","isBuiltIn":false,"description":"Current","createdAt":0,"updatedAt":0,
            "accessOwnerID":"cli-local-user","accessRevision":1,"accessGrants":[]});
        let snapshot = json!({"customTemplates":[template.clone()],"selectedTemplateID":null});
        let value = project_for_host(3, &snapshot.to_string()).unwrap();
        assert_eq!(
            value,
            json!({"revision":3,"templates":[{"id":id,"name":"Expert rules"}]})
        );
        assert!(project_for_host(
            3,
            &json!({"customTemplates":[template.clone(),template]}).to_string()
        )
        .is_err());
        assert!(project_for_host(3, &json!({"templates":[]}).to_string()).is_err());
        assert!(project_for_host(0, &snapshot.to_string()).is_err());
        let mut builtin = snapshot.clone();
        builtin["customTemplates"][0]["isBuiltIn"] = json!(true);
        assert!(project_for_host(3, &builtin.to_string()).is_err());
        let mut private = snapshot.clone();
        private["customTemplates"][0]["accessOwnerID"] = json!("other-user");
        assert_eq!(
            project_for_host(3, &private.to_string()).unwrap()["templates"],
            json!([])
        );
        private["customTemplates"][0]["accessGrants"] = json!([{"grant_id":"read-for-host", "resource_kind":"prompt_template",
            "resource_id":id.to_lowercase(), "principal":{"kind":"local_user","id":"cli-local-user"},
            "permissions":["read_metadata"],"resource_revision":1}]);
        assert_eq!(
            project_for_host(3, &private.to_string()).unwrap()["templates"],
            json!([])
        );
        private["customTemplates"][0]["accessGrants"][0]["permissions"] = json!(["read_content"]);
        assert_eq!(
            project_for_host(3, &private.to_string()).unwrap()["templates"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        private["customTemplates"][0]["accessGrants"][0]["resource_revision"] = json!(2);
        assert_eq!(
            project_for_host(3, &private.to_string()).unwrap()["templates"],
            json!([])
        );
        let mut unowned = snapshot.clone();
        unowned["customTemplates"][0]
            .as_object_mut()
            .unwrap()
            .remove("accessOwnerID");
        assert_eq!(
            project_for_host(3, &unowned.to_string()).unwrap()["templates"],
            json!([])
        );
    }
}
