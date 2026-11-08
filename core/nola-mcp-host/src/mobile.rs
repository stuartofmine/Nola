// SPDX-License-Identifier: Apache-2.0
//! Embedded HTTP Host adapter. No shell/process, credentials or permission rules.
//! Core admits every request; current configuration and owner fence transport.
use crate::{catalog, mcp_connection_authority as authority, mcp_host as transport, CancellationToken};
use nola_core::agent::{capability::McpTransportKind, host::HostCapabilityDescriptor};
use serde_json::{json, Value};
use std::{collections::{BTreeMap, BTreeSet}, sync::{Mutex, OnceLock}};

type Owner = (String, String);
#[derive(Default)]
struct Registry {
    configurations: BTreeMap<Owner, Vec<Value>>,
    policies:BTreeMap<Owner,nola_core::tool_policy::ToolPolicyInput>,
    calls: BTreeMap<(String, String, String), CancellationToken>,
    cancelled: BTreeSet<(String, String, String)>,
}
fn registry() -> &'static Mutex<Registry> { static VALUE: OnceLock<Mutex<Registry>>=OnceLock::new(); VALUE.get_or_init(Default::default) }
fn owner(input: &Value) -> Result<Owner,String> {
    let host=input["host_id"].as_str().filter(|s|!s.is_empty()).ok_or("Missing MCP Host")?;
    let session=input["session_id"].as_str().filter(|s|!s.is_empty()).ok_or("Missing MCP session")?;
    Ok((host.into(),session.into()))
}
fn descriptors(owner: &Owner) -> Result<BTreeMap<String,HostCapabilityDescriptor>,String> {
    let configs=registry().lock().map_err(|_|"MCP registry unavailable")?.configurations.get(owner).cloned().unwrap_or_default();
    let mut out=BTreeMap::new();
    let refs=configs.iter().map(|v|v["connection_ref"].clone()).collect::<Vec<_>>();
    if !refs.is_empty() { let d=catalog::connect_descriptor(configs.clone());out.insert(d.id.clone(),d); }
    for value in transport::describe_connected_cache(&configs).map_err(|e|e.message)? {
        let server:nola_core::agent::capability::McpServerDescriptor=serde_json::from_value(value).map_err(|e|e.to_string())?;
        if server.availability != nola_core::agent::capability::McpServerAvailability::Available {continue;}
        for tool in server.tools.values() {let d=catalog::tool_descriptor(&server,tool);out.insert(d.id.clone(),d);}
    }
    let policy=registry().lock().map_err(|_|"MCP registry unavailable")?.policies.get(owner).cloned();
    out.retain(|_,d|policy.as_ref().is_some_and(|p| nola_core::tool_policy::mcp_host_descriptor_allowed(p,d)));
    Ok(out)
}
fn prepared(owner:&Owner) -> Result<Value,String> {
    let caps=descriptors(owner)?;
    let tools=caps.values().filter(|d| d.id==authority::CONNECT_CAPABILITY).map(|d|json!({"type":"function","function":{"name":d.id,"description":d.input_schema.get("description").unwrap_or(&json!(d.title)),"parameters":d.input_schema}})).collect::<Vec<_>>();
    Ok(json!({"capabilities":caps,"tools":tools,"permission_rules":[
        {"capability_pattern":"mcp_connection","resource_pattern":"*","decision":"ask","source":"host"},
        {"capability_pattern":"mcp","resource_pattern":"*","decision":"ask","source":"host"},
        {"capability_pattern":"mcp_destructive","resource_pattern":"*","decision":"ask","source":"host"}
    ]}))
}
/// Configuration projection performs no network I/O. stdio is deliberately not
/// installed on embedded mobile Hosts; a connected desktop Host owns its processes.
pub fn command(input:Value) -> Result<Value,String> {
    let owner=owner(&input)?;
    match input["type"].as_str() {
        Some("prepare") => {
            let mut policies=registry().lock().map_err(|_|"MCP registry unavailable")?;
            policies.policies.remove(&owner);
            if let Some(raw)=input.get("policy").filter(|v|!v.is_null()) {
                let mut policy:nola_core::tool_policy::ToolPolicyInput=serde_json::from_value(raw.clone()).map_err(|e|e.to_string())?;
                policy.now_ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_millis() as u64;
                if policy.expected_identity.host_id.as_deref()==Some(owner.0.as_str()) && policy.expected_identity.session_id.as_deref()==Some(owner.1.as_str()) {policies.policies.insert(owner.clone(),policy);}
            }
            drop(policies);
            let mut configs=Vec::new();
            for record in input["servers"].as_array().ok_or("Missing MCP configuration records")? {
                if record["enabled"]!=true || !record["deleted_at_ms"].is_null() {continue;}
                let mut config:Value=serde_json::from_str(record["configuration_json"].as_str().ok_or("Invalid MCP configuration")?).map_err(|e|e.to_string())?;
                if crate::mcp_stdio::is_stdio_configuration(&config) {continue;}
                let fields=config.as_object_mut().ok_or("Invalid MCP configuration object")?;
                fields.insert("server_id".into(),record["id"].clone());
                fields.insert("display_name".into(),record["display_name"].clone());
                fields.insert("enabled".into(),json!(true));
                configs.push(config);
            }
            let bound=authority::bind_configurations(&configs,&owner.0,&owner.1)?;
            let refs=bound.iter().map(|v|v["connection_ref"].as_str().unwrap_or_default().to_owned()).collect::<BTreeSet<_>>();
            let old=registry().lock().map_err(|_|"MCP registry unavailable")?.configurations.insert(owner.clone(),bound.clone()).unwrap_or_default();
            for removed in old.iter().filter_map(|v|v["connection_ref"].as_str()).filter(|r|!refs.contains(*r)) {
                let retirement=transport::revoke_connection(McpTransportKind::StreamableHttp,removed).map_err(|e|e.message)?;
                std::thread::spawn(move||drop(retirement));
            }
            transport::register_configured_servers(&bound).map_err(|e|e.message)?;
            prepared(&owner)
        }
        Some("catalog") => prepared(&owner),
        Some("cancel") => {
            let call=input["call_id"].as_str().ok_or("Missing MCP call identity")?;
            let key=(owner.0,owner.1,call.into());
            let mut r=registry().lock().map_err(|_|"MCP registry unavailable")?;
            r.cancelled.insert(key.clone());
            if let Some(token)=r.calls.get(&key) {token.request();}
            Ok(Value::Null)
        }
        Some("execute") => execute(&owner,&input["request"],&input["admitted_capability"]),
        _=>Err("Unsupported embedded MCP Host command".into())
    }
}
fn result(call:&str,data:Value,state:&str,error:bool,activation:Value,artifacts:Value)->Value {
    json!({"type":"tool_completed","call_id":call,
        "output":{"contract":"nola.tool_result.v1","data":data,"presentation":{"text":"","summary":""},
          "execution":{"state":state,"local_stop":null,"effects":[],"checks":[]},"full_output_ref":null,"memory_usage":null},
        "outcome":if error {"failed"} else {"succeeded"},"activation":activation,"full_output_ref":null,"artifacts":artifacts})
}
fn execute(owner:&Owner,request:&Value,admitted:&Value)->Result<Value,String> {
    if request["host_id"]!=owner.0 || request["session_id"]!=owner.1 {return Err("MCP request belongs to another Host/session".into());}
    let payload=&request["payload"];
    if payload["type"]!="execute_tool" {return Err("Not a Core tool execution request".into());}
    let id=payload["capability_id"].as_str().ok_or("Missing capability identity")?;
    let call=payload["call_id"].as_str().ok_or("Missing call identity")?;
    let token=CancellationToken::default();let key=(owner.0.clone(),owner.1.clone(),call.into());
    {let mut r=registry().lock().map_err(|_|"MCP registry unavailable")?;
        if r.calls.contains_key(&key){return Err("MCP call is already executing".into());}
        if r.cancelled.contains(&key){token.request();}
        r.calls.insert(key.clone(),token.clone());}
    struct CallGuard((String,String,String));
    impl Drop for CallGuard {fn drop(&mut self){if let Ok(mut r)=registry().lock(){r.calls.remove(&self.0);}}}
    let _call=CallGuard(key);
    if token.is_requested(){return Ok(result(call,json!({"error":"mcp_cancelled_before_send"}),"not_started",true,Value::Null,json!([])));}
    let caps=descriptors(owner)?;
    let Some(descriptor)=caps.get(id) else {return Ok(result(call,json!({"error":"mcp_configuration_unavailable","message":"MCP server was disabled, changed, or not connected. A new connection requires approval."}),"not_started",true,Value::Null,json!([])));};
    if &serde_json::to_value(descriptor).map_err(|e|e.to_string())? != admitted {
        return Ok(result(call,json!({"error":"mcp_catalog_changed","message":"Current MCP descriptor differs from Core admission; refresh the catalog before retrying."}),"not_started",true,Value::Null,json!([])));
    }
    let args=&payload["input"];
    if id==authority::CONNECT_CAPABILITY {
        let reference=args["connection_ref"].as_str().ok_or("Missing configured MCP connection")?;
        let allowed=descriptor.input_schema.pointer("/properties/connection_ref/enum").and_then(Value::as_array).is_some_and(|refs|refs.iter().any(|r|r==reference));
        if !allowed {return Err("MCP connection is outside current configuration".into());}
        let expected=authority::permission_resource(reference)?;
        if expected.host_id!=owner.0 || expected.session_id!=owner.1 {return Err("MCP connection owner mismatch".into());}
        return match transport::connect_approved(reference,&expected) {
            Ok(connected)=>{
                let admitted=descriptors(owner)?;
                let descriptors=connected.server.tools.values().filter_map(|t|admitted.get(&catalog::tool_id(&connected.server,t)).cloned()).collect::<Vec<_>>();
                let ids=descriptors.iter().map(|d|d.id.clone()).collect::<Vec<_>>();
                let activation=json!({"source":"mcp_connection","capability_ids":ids,"capability_descriptors":descriptors});
                let mut reply=result(call,json!({"server_id":connected.server.server_id,"connected":true,"capability_ids":ids,"authority":"Connection only; every tool call requires independent Core permission evaluation."}),"completed",false,activation,json!([]));
                let generation=connected.resource.generation.ok_or("MCP connection has no generation")?;
                let uri=format!("mcp-connection://{}",reference.trim_start_matches("mcp:"));
                if expected.generation.is_none(){reply["output"]["execution"]["effects"]=json!([{"resource_uri":uri,"operation":"connect","before_version":null,"after_version":generation}]);}
                reply["output"]["execution"]["checks"]=json!([{"kind":"postcondition","resource_uri":uri,"version":generation,"predicate":"Host retained an initialized MCP client and discovered descriptor for this exact owner/configuration","passed":true}]);
                Ok(reply)
            },
            Err(e)=>Ok(result(call,json!({"error":e.code,"message":e.message}),"unknown",true,Value::Null,json!([])))
        };
    }
    let route=descriptor.input_normalization.mcp_route.as_ref().ok_or("Not an MCP tool route")?;
    let resource=authority::permission_resource(&route.connection_ref)?;
    if resource.host_id!=owner.0 || resource.session_id!=owner.1 || resource.config_digest!=route.config_digest || resource.generation.is_none(){return Err("MCP route is not the current initialized owner/configuration".into());}
    let executed=transport::call_tool_with_cancellation(McpTransportKind::StreamableHttp,&route.connection_ref,&route.tool_name,args.clone(),&token);
    Ok(match executed {
        Ok(value)=>result(call,value.output,"completed",value.is_error,Value::Null,serde_json::to_value(value.artifacts).map_err(|e|e.to_string())?),
        Err(e)=>{let state=if matches!(e.code,"mcp_cancelled_before_send"|"mcp_connection_required"|"mcp_connection_unavailable"|"mcp_connection_retired") {"not_started"}else{"unknown"};result(call,json!({"error":e.code,"message":e.message}),state,true,Value::Null,json!([]))}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mobile_metadata_disable_and_owner_fence_never_connect() {
        let host=uuid::Uuid::new_v4().to_string();let session=uuid::Uuid::new_v4().to_string();
        let server=json!({"id":"own-server","display_name":"Own fixture","enabled":true,"deleted_at_ms":null,"configuration_json":r#"{"url":"http://127.0.0.1:1/mcp"}"#});
        let prepared=command(json!({"type":"prepare","host_id":host,"session_id":session,"servers":[server]})).unwrap();
        assert_eq!(prepared["capabilities"],json!({}));
        assert_eq!(prepared["permission_rules"][0]["decision"],"ask");
        let frozen=super::catalog::connect_descriptor(vec![]);let frozen=serde_json::to_value(frozen).unwrap();
        let request=json!({"host_id":host,"session_id":session,"payload":{"type":"execute_tool","capability_id":"mcp_connect","call_id":"c","input":{"connection_ref":frozen["input_schema"]["properties"]["connection_ref"]["enum"][0]}}});
        let foreign=json!({"type":"execute","host_id":"other","session_id":session,"request":request,"admitted_capability":frozen});
        assert!(command(foreign).unwrap_err().contains("another Host/session"));
        let disabled=command(json!({"type":"prepare","host_id":host,"session_id":session,"servers":[]})).unwrap();
        assert_eq!(disabled["capabilities"],json!({}));
        let result=command(json!({"type":"execute","host_id":host,"session_id":session,"request":request,"admitted_capability":frozen})).unwrap();
        assert_eq!(result["output"]["execution"]["state"],"not_started");
        assert_eq!(result["outcome"],"failed");
        let stdio=json!({"id":"process","display_name":"Unsupported on mobile","enabled":true,"configuration_json":r#"{"transport":"stdio","command":"must-not-launch"}"#});
        let prepared=command(json!({"type":"prepare","host_id":host,"session_id":session,"servers":[stdio]})).unwrap();
        assert_eq!(prepared["capabilities"],json!({}));
    }
}
