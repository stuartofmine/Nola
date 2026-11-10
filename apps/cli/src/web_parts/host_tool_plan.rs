// SPDX-License-Identifier: Apache-2.0
// Host-issued preparation is durable until its original expiry; admitted snapshots belong to the Run transaction.
use nola_domain::host_tool_plan::{HostToolPlanSnapshot, HostToolPlanReference, HostToolPlanTool};

const HOST_TOOL_PLAN_MAX_BYTES: usize = 512 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HostToolPlanPrepare {
    expected_host_id: String,
    client_id: String,
    session_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HostToolPlanRead {
    expected_host_id: String,
    client_id: String,
    session_id: String,
    expected_run_id: String,
    assistant_message_id: String,
}

fn host_tool_plan_now() -> u64 { unix_time_seconds().saturating_mul(1_000) }

fn host_tool_plan_tools(settings: &AgentSettings, skills: &crate::skill_runtime::SkillRuntimeContext, session: &str)
    -> Result<(Vec<HostToolPlanTool>, String), String> {
    use sha2::{Digest, Sha256};
    let mut schemas = resolve_web_agent_host_tools_from_catalog(settings, &[], &crate::tools::tool_schemas_with_skills(skills));
    schemas.push(nola_core::agent::generative_ui::tool_schema());
    let registry = crate::resource_access::mcp_runtime_resources(&settings.mcp_servers);
    let registry = crate::mcp_connection_authority::bind_configurations(&registry,"nola-cli-host",session)?;
    crate::mcp_host::register_configured_servers(&registry).map_err(|error|error.message)?;
    let registry = crate::mcp_host::describe_connected_cache(&registry).map_err(|error|error.message)?;
    let catalog = crate::agent_host_driver::configured_mcp_capability_catalog(&registry,
        crate::agent_host_driver::configured_mcp_capability_catalog_revision(&registry))?;
    let generations = catalog.servers.values().map(|server| {
        crate::mcp_connection_authority::permission_resource(&server.connection_ref)
    }).collect::<Result<Vec<_>,_>>()?;
    schemas.extend(crate::agent_host_driver::remote_mcp_tool_schemas(&catalog));
    let mut tools = std::collections::BTreeMap::new();
    for schema in schemas {
        let id = schema.pointer("/function/name").and_then(Value::as_str).ok_or("Host tool has no name")?.to_owned();
        let item = HostToolPlanTool { tool_id:id.clone(),route:"agent_host".into(),schema };
        if let Some(previous) = tools.insert(id, item.clone()) {
            if previous != item { return Err("Host catalog contains conflicting schemas".into()); }
        }
    }
    let tools: Vec<_> = tools.into_values().collect();
    // MCP descriptors bind configured connection generation and resource/prompt
    // catalogs too. Only the digest leaves the Host, never connection settings.
    let bytes = serde_json::to_vec(&json!({"tools":tools,"mcp":catalog,"generations":generations})).map_err(|e|e.to_string())?;
    let digest = format!("{:x}",Sha256::digest(bytes));
    Ok((tools,digest))
}

fn capture_current_host_tool_plan_tools(config: &Config, session: &str) -> Result<(Vec<HostToolPlanTool>,String),String> {
    let settings = NolaStore::open_default().and_then(|store| load_agent_settings_from_store(&store))?;
    let skills = web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings)?;
    host_tool_plan_tools(&settings,&skills,session)
}

fn host_tool_plan_prepare(input: HostToolPlanPrepare, config: &Config) -> Result<HostToolPlanSnapshot,String> {
    let host = host_identity();
    if input.expected_host_id != host || [&input.client_id,&input.session_id].iter()
        .any(|v|v.trim().is_empty() || v.len()>512) { return Err("Host tool plan identity mismatch".into()); }
    let (tools,catalog_revision) = capture_current_host_tool_plan_tools(config,&input.session_id)?;
    let now = host_tool_plan_now();
    let snapshot = HostToolPlanSnapshot { schema_version:1,reference:HostToolPlanReference {
        manifest_id:uuid::Uuid::new_v4().to_string(),host_id:host.to_owned(),client_id:input.client_id,session_id:input.session_id,catalog_revision },
        issued_at_ms:now,expires_at_ms:now+300_000,tools };
    snapshot.validate(host,&snapshot.reference.client_id,&snapshot.reference.session_id,Some(now))?;
    if serde_json::to_vec(&snapshot).map_err(|e|e.to_string())?.len()>HOST_TOOL_PLAN_MAX_BYTES {
        return Err("Host tool plan exceeds transport budget".into());
    }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let _: () = application.execute(nola_application::ApplicationCommand::PersistPreparedHostToolPlan {
        snapshot: snapshot.clone(), now_ms: now,
    })?;
    Ok(snapshot)
}

fn admit_host_tool_plan(body: &Value, config: &Config) -> Result<HostToolPlanSnapshot,String> {
    for field in ["tool_policy","policy_digest","manifest_digest","manifest_valid_until_ms"] {
        if body.get(field).is_some() { return Err(format!("Unsupported client-generated tool policy field: {field}")); }
    }
    let reference: HostToolPlanReference = serde_json::from_value(body.get("tool_plan_ref").cloned().ok_or("Host tool plan reference required")?)
        .map_err(|e|e.to_string())?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let plan: Option<HostToolPlanSnapshot> = application.execute(nola_application::ApplicationCommand::ReadPreparedHostToolPlan {
        reference: reference.clone(), now_ms: host_tool_plan_now(),
    })?;
    let plan = plan.ok_or("Host tool plan is missing")?;
    if plan.reference != reference { return Err("Host tool plan reference mismatch".into()); }
    let client = body["client_id"].as_str().ok_or("Host client identity required")?;
    let session = body["conversationId"].as_str().ok_or("Host session identity required")?;
    plan.validate(host_identity(),client,session,Some(host_tool_plan_now()))?;
    if body["expected_host_id"].as_str()!=Some(host_identity()) { return Err("Host identity mismatch".into()); }
    let (_,revision) = capture_current_host_tool_plan_tools(config,session)?;
    if revision != plan.reference.catalog_revision { return Err("Host tool catalog changed; prepare again".into()); }
    Ok(plan)
}

fn read_run_host_tool_plan(input: &HostToolPlanRead) -> Result<HostToolPlanSnapshot,String> {
    if input.expected_host_id != host_identity() { return Err("Host identity mismatch".into()); }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let exact: Option<nola_application::ExactRunInvocation> = application.execute(nola_application::ApplicationCommand::LoadExactRunInvocation {
        conversation_id:input.session_id.clone(),expected_run_id:input.expected_run_id.clone(),assistant_message_id:input.assistant_message_id.clone() })?;
    let snapshot = exact.ok_or("Run not found")?.remote_tool_plan.ok_or("Run has no remote Host tool plan")?;
    snapshot.validate(&input.expected_host_id,&input.client_id,&input.session_id,None)?;
    Ok(snapshot)
}

fn host_tool_plan_response(request: &mut Request, config: &Config, mobile: bool, read: bool) -> Response<Box<dyn Read + Send>> {
    let token = if mobile { match mobile_bearer_token(request) { Some(token)=>Some(token),None=>return json_response(StatusCode(401),json!({"error":"unauthorized"})) } } else {None};
    let body = match token.as_deref() { Some(token)=>read_mobile_json_value(request,token),None=>read_json_value(request) };
    let body = match body {Ok(body)=>body,Err(response)=>return response};
    let result = if read { serde_json::from_value::<HostToolPlanRead>(body).map_err(|e|e.to_string()).and_then(|input|read_run_host_tool_plan(&input)) }
        else { serde_json::from_value::<HostToolPlanPrepare>(body).map_err(|e|e.to_string()).and_then(|input|host_tool_plan_prepare(input,config)) };
    let (status,value) = match result {
        Ok(snapshot)=>(StatusCode(200),json!({"source_host_id":host_identity(),"result":snapshot})),
        Err(error)=>(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":error})),
    };
    match token {Some(token)=>mobile_encrypted_json_response(status,value,&token),None=>json_response(status,value)}
}

#[cfg(test)]
mod host_tool_plan_catalog_tests {
    use super::*;
    #[test]
    fn network_preparation_rejects_client_issued_snapshots() {
        let input = json!({"expected_host_id":"host","client_id":"client","session_id":"context"});
        assert!(serde_json::from_value::<HostToolPlanPrepare>(input.clone()).is_ok());
        for field in ["snapshot", "tools", "expires_at_ms", "reference", "catalog_revision"] {
            let mut forged = input.clone();
            forged[field] = json!({});
            assert!(serde_json::from_value::<HostToolPlanPrepare>(forged).is_err(), "{field}");
        }
    }
    #[test]
    fn configured_mcp_preparation_uses_owned_metadata_without_connecting() {
        let mut settings=AgentSettings::default();
        settings.mcp_servers=crate::resource_access::reconcile_mcp_resources(&[],&[json!({"server_id":"plan-fixture","name":"plan-fixture",
            "endpoint":"http://127.0.0.1:1/mcp","enabled":true})]).unwrap();
        let skills=crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        let session=uuid::Uuid::new_v4().to_string();
        let (tools,first)=host_tool_plan_tools(&settings,&skills,&session).unwrap();
        let (_,second)=host_tool_plan_tools(&settings,&skills,&session).unwrap();
        assert_eq!(first,second,"preparation must preserve cache generation");
        let connect=tools.iter().find(|tool|tool.tool_id==crate::mcp_connection_authority::CONNECT_CAPABILITY).unwrap();
        let reference=connect.schema.pointer("/function/parameters/properties/connection_ref/enum/0").and_then(Value::as_str).unwrap();
        let resource=crate::mcp_connection_authority::permission_resource(reference).unwrap();
        assert_eq!(resource.session_id,session);
        assert!(resource.generation.is_none(),"preparation must never connect");
        let (_,other)=host_tool_plan_tools(&settings,&skills,&uuid::Uuid::new_v4().to_string()).unwrap();
        assert_ne!(first,other,"connection ownership is part of revision");
    }
}
