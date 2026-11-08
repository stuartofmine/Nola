// SPDX-License-Identifier: Apache-2.0
//! Host-owned MCP connection identity. Configuration and discovery grant no authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const CONNECT_CAPABILITY: &str = "mcp_connect";
const OWNER_FIELD: &str = "_nola_mcp_connection";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionResource {
    pub host_id: String,
    pub session_id: String,
    pub connection_ref: String,
    pub config_digest: String,
    pub transport: String,
    pub target: String,
    /// Host-private incarnation/attempt fence; never a model-provided handle.
    pub admission_id: String,
    pub generation: Option<String>,
}

#[derive(Debug)]
pub struct ConnectionEstablished {
    pub server: nola_core::agent::capability::McpServerDescriptor,
    pub resource: ConnectionResource,
}

pub fn resource_from_configuration(value: &Value) -> Result<ConnectionResource, String> {
    let resource: ConnectionResource = serde_json::from_value(value.get(OWNER_FIELD).cloned()
        .ok_or("MCP configuration has no bound Host/session identity")?).map_err(|error|error.to_string())?;
    if resource.host_id.trim().is_empty() || resource.session_id.trim().is_empty()
        || resource.connection_ref != value["connection_ref"].as_str().unwrap_or("")
        || resource.generation.is_some() || !resource.admission_id.is_empty() {
        return Err("MCP configuration owner differs from its registered resource".into());
    }
    Ok(resource)
}

/// Freeze launch inputs and bind them to one concrete Host/session before any
/// registry insertion. Never trust an owner field supplied in editable config.
pub fn bind_configurations(values: &[Value], host: &str, session: &str) -> Result<Vec<Value>,String> {
    if host.trim().is_empty() || session.trim().is_empty() { return Err("MCP requires a concrete Host/session owner".into()); }
    values.iter().enumerate().map(|(index,value)| {
        if value["enabled"] == false { return Ok(value.clone()); }
        let server = crate::mcp_host::configured_server_id(value,index).map_err(|error|error.message)?;
        let mut configured = value.as_object().cloned().ok_or("MCP configuration must be an object")?;
        configured.remove(OWNER_FIELD);
        let (transport,target,config_digest) = if crate::mcp_stdio::is_stdio_configuration(value) {
            let (target,digest) = crate::mcp_stdio::freeze_configuration(value,&server).map_err(|error|error.message)?;
            ("stdio",target,digest)
        } else {
            let endpoint = value.get("endpoint").or_else(|| value.get("url"))
                .and_then(Value::as_str).ok_or("MCP HTTP endpoint is missing")?;
            let url = url::Url::parse(endpoint).map_err(|_|"MCP endpoint is invalid")?;
            if !matches!(url.scheme(),"http"|"https") || !url.username().is_empty() || url.password().is_some() {
                return Err("MCP endpoint must be HTTP(S), with credentials in Host headers only".into());
            }
            let identity = json!({"endpoint":url.as_str(),"headers":value.get("headers").cloned().unwrap_or(json!({})),
                "timeout_ms":value.get("timeout_ms").and_then(Value::as_u64).unwrap_or(15_000).clamp(1_000,120_000)});
            ("streamable_http",url.origin().ascii_serialization(),format!("{:x}",Sha256::digest(
                serde_json::to_vec(&identity).map_err(|error|error.to_string())?)))
        };
        let connection_ref = format!("mcp:{}",format!("{:x}",Sha256::digest(
            serde_json::to_vec(&json!([host,session,server,config_digest])).map_err(|error|error.to_string())?)));
        let resource = ConnectionResource {host_id:host.into(),session_id:session.into(),connection_ref:connection_ref.clone(),
            config_digest,transport:transport.into(),target,admission_id:String::new(),generation:None};
        configured.insert("connection_ref".into(),json!(connection_ref));
        configured.insert(OWNER_FIELD.into(),serde_json::to_value(resource).map_err(|error|error.to_string())?);
        Ok(Value::Object(configured))
    }).collect()
}

pub fn permission_resource(reference: &str) -> Result<ConnectionResource,String> {
    crate::mcp_host::connection_resource(reference)
        .map_err(|error|error.message)
}

#[cfg(any(test, feature = "test-support"))]
pub fn connect_fixture_servers(values: &[Value]) -> Result<Vec<Value>,crate::mcp_host::McpHostError> {
    connect_fixture_servers_owned(values,"fixture-host",&uuid::Uuid::new_v4().to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn connect_fixture_servers_owned(values:&[Value],host:&str,session:&str) -> Result<Vec<Value>,crate::mcp_host::McpHostError> {
    let owned = bind_configurations(values,host,session)
        .map_err(|message|crate::mcp_host::McpHostError {code:"mcp_fixture_identity",message})?;
    crate::mcp_host::register_configured_servers(&owned)?;
    for value in &owned {
        if value["enabled"] == false { continue; }
        let reference = value["connection_ref"].as_str().unwrap();
        let expected = permission_resource(reference).map_err(|message|crate::mcp_host::McpHostError {code:"mcp_fixture_identity",message})?;
        crate::mcp_host::connect_approved(reference,&expected)?;
    }
    crate::mcp_host::describe_connected_cache(&owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read,Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[test]
    fn mcp_metadata_and_refresh_do_not_connect_and_owner_changes_do_not_reuse() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = vec![json!({"server_id":"lazy-fixture","endpoint":format!("http://{}/mcp",listener.local_addr().unwrap())})];
        let session = uuid::Uuid::new_v4().to_string();
        let owned = bind_configurations(&config,"host-a",&session).unwrap();
        let mut external = config.clone();
        external[0]["url"] = external[0]["endpoint"].take();
        external[0].as_object_mut().unwrap().remove("endpoint");
        assert_eq!(owned[0]["connection_ref"], bind_configurations(&external,"host-a",&session).unwrap()[0]["connection_ref"]);
        crate::mcp_host::register_configured_servers(&owned).unwrap();
        let reference = owned[0]["connection_ref"].as_str().unwrap();
        let expected = permission_resource(reference).unwrap();
        assert_eq!(expected.generation,None);
        assert_eq!(crate::mcp_host::describe_configured_servers(&config).unwrap()[0]["availability"],"authorization_required");
        assert_eq!(crate::mcp_host::describe_connected_cache(&owned).unwrap()[0]["availability"],"authorization_required");
        assert!(crate::mcp_host::refresh_changed_servers(&owned).unwrap().is_none());
        crate::mcp_host::discover_connected_servers(&owned).unwrap();
        let mut foreign = expected.clone(); foreign.session_id = "foreign".into();
        assert_eq!(crate::mcp_host::connect_approved(reference,&foreign).unwrap_err().code,"mcp_connection_changed");
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        assert_ne!(owned[0]["connection_ref"],bind_configurations(&config,"host-b",&session).unwrap()[0]["connection_ref"]);
        assert_ne!(owned[0]["connection_ref"],bind_configurations(&config,"host-a","another-session").unwrap()[0]["connection_ref"]);
        let mut changed = config.clone(); changed[0]["headers"] = json!({"Authorization":"fixture-only"});
        assert_ne!(owned[0]["connection_ref"],bind_configurations(&changed,"host-a",&session).unwrap()[0]["connection_ref"]);
    }

    #[test]
    fn mcp_explicit_connect_retains_generation_and_loss_does_not_reconnect() {
        // Exercise ambient proxy rejection in an isolated test process, never
        // race other tests by changing this process's global environment.
        if std::env::var_os("NOLA_MCP_PROXY_TEST_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","mcp_connection_authority::tests::mcp_explicit_connect_retains_generation_and_loss_does_not_reconnect","--nocapture"])
                .env("NOLA_MCP_PROXY_TEST_CHILD","1")
                .env("HTTP_PROXY","http://127.0.0.1:1").env("http_proxy","http://127.0.0.1:1")
                .env("HTTPS_PROXY","http://127.0.0.1:1").env("https_proxy","http://127.0.0.1:1")
                .env("ALL_PROXY","http://127.0.0.1:1").env("all_proxy","http://127.0.0.1:1")
                .env("NO_PROXY","").env("no_proxy","").status().unwrap();
            assert!(status.success(),"owner-bound MCP must not inherit ambient proxy routing");
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = vec![json!({"server_id":"retained-fixture","endpoint":format!("http://{}/mcp",listener.local_addr().unwrap()),"timeout_ms":1000})];
        let owned = bind_configurations(&config,"retained-host",&uuid::Uuid::new_v4().to_string()).unwrap();
        crate::mcp_host::register_configured_servers(&owned).unwrap();
        let reference = owned[0]["connection_ref"].as_str().unwrap();
        let expected = permission_resource(reference).unwrap();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now()+Duration::from_secs(10);
            let mut count = 0;
            while count < 6 {
                let (mut stream,_) = match listener.accept() {
                    Ok(value)=>value,
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now()<deadline,"MCP fixture did not receive all handshake requests");
                        std::thread::sleep(Duration::from_millis(5)); continue;
                    }
                    Err(error)=>panic!("MCP accept failed: {error}"),
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new(); let mut buffer = [0u8;4096];
                let head = loop {let n=stream.read(&mut buffer).unwrap(); assert!(n>0); bytes.extend_from_slice(&buffer[..n]);
                    if let Some(at)=bytes.windows(4).position(|part|part==b"\r\n\r\n") {break at+4;}};
                let length = String::from_utf8_lossy(&bytes[..head]).lines().find_map(|line| {
                    let (name,value)=line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(||value.trim().parse::<usize>().unwrap())
                }).unwrap();
                while bytes.len()<head+length {let n=stream.read(&mut buffer).unwrap();assert!(n>0);bytes.extend_from_slice(&buffer[..n]);}
                let request:Value=serde_json::from_slice(&bytes[head..head+length]).unwrap();
                let result=match request["method"].as_str().unwrap() {
                    "initialize"=>json!({"protocolVersion":"2025-06-18","capabilities":{}}),
                    "tools/list"=>json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}),
                    "resources/list"=>json!({"resources":[]}),
                    "resources/templates/list"=>json!({"resourceTemplates":[]}),
                    "prompts/list"=>json!({"prompts":[]}),
                    "notifications/initialized"=>Value::Null,
                    method=>panic!("unexpected MCP request {method}"),
                };
                let body=if request.get("id").is_some() {json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()} else {String::new()};
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                count+=1;
            }
            count
        });
        let connected=crate::mcp_host::connect_approved(reference,&expected).unwrap();
        assert_eq!(server.join().unwrap(),6);
        assert!(connected.resource.generation.is_some());
        let reused=crate::mcp_host::connect_approved(reference,&connected.resource).unwrap();
        assert_eq!(reused.resource,connected.resource);
        assert!(crate::mcp_host::call_tool(nola_core::agent::capability::McpTransportKind::StreamableHttp,reference,"echo",json!({})).is_err());
        assert!(permission_resource(reference).unwrap().generation.is_none());
        assert_ne!(permission_resource(reference).unwrap(),expected,
            "the initial pending approval cannot authorize a reconnect after loss");
        assert_eq!(crate::mcp_host::connect_approved(reference,&expected).unwrap_err().code,"mcp_connection_changed");
        assert!(crate::mcp_host::refresh_changed_servers(&owned).unwrap().is_none());
        assert_eq!(crate::mcp_host::call_tool(nola_core::agent::capability::McpTransportKind::StreamableHttp,reference,"echo",json!({})).unwrap_err().code,"mcp_connection_required");
    }
}
