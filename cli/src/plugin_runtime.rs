// SPDX-License-Identifier: Apache-2.0
//! Worker-owned plugin MCP lifecycle. Only typed descriptors leave this module;
//! launch configuration and credentials remain in the existing Host transport.
use nola_core::agent::capability::{McpServerDescriptor, McpTransportKind};
use nola_domain::plugin::PluginInstallation;
use nola_plugin_host::{PluginRuntime, RuntimeOwner, RuntimeSkill};

type InstallationReader = Box<dyn FnMut(&str) -> Result<Option<PluginInstallation>, String> + Send>;

#[derive(Clone)]
pub struct SharedPluginSession(std::sync::Arc<std::sync::Mutex<PluginMcpSession>>);
impl std::fmt::Debug for SharedPluginSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("SharedPluginSession(..)") }
}
impl PartialEq for SharedPluginSession {
    fn eq(&self, other: &Self) -> bool { std::sync::Arc::ptr_eq(&self.0, &other.0) }
}
impl Eq for SharedPluginSession {}
impl From<PluginMcpSession> for SharedPluginSession {
    fn from(value: PluginMcpSession) -> Self { Self(std::sync::Arc::new(std::sync::Mutex::new(value))) }
}
impl SharedPluginSession {
    pub(crate) fn configurations(&self) -> Result<Vec<serde_json::Value>,String> {
        let mut session = self.0.lock().map_err(|_| "plugin session unavailable")?;
        session.reconcile()?;
        Ok(session.runtimes.iter().filter(|runtime|!runtime.retired).flat_map(|runtime|
            runtime.prepared.host_servers().iter().cloned().map(|mut value| {
                value["_nola_plugin_registration_ref"] = value["connection_ref"].clone(); value
            })).collect())
    }
    pub(crate) fn bind_connections(&self, values:&[serde_json::Value]) -> Result<(),String> {
        let mut session = self.0.lock().map_err(|_| "plugin session unavailable")?;
        session.reconcile()?;
        for runtime in &mut session.runtimes {
            if runtime.retired { continue; }
            let matching = values.iter().filter(|value|runtime.prepared.host_servers().iter()
                .any(|original|original["connection_ref"] == value["_nola_plugin_registration_ref"])).cloned().collect::<Vec<_>>();
            for value in matching {
                if !runtime.bound_configurations.iter().any(|existing|existing["connection_ref"] == value["connection_ref"]) {
                    runtime.bound_configurations.push(value);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn revisions(&self) -> Result<std::collections::BTreeMap<String, i64>, String> {
        let mut session = self.0.lock().map_err(|_| "plugin session unavailable")?;
        session.reconcile()?;
        Ok(session.runtimes.iter().filter(|runtime| !runtime.retired)
            .map(|runtime| (runtime.owner().plugin_id.clone(), runtime.owner().revision)).collect())
    }
    pub fn reconcile(&self) -> Result<(), String> {
        self.0.lock().map_err(|_| "plugin session unavailable")?.reconcile()
    }
    pub fn contains_connection(&self, reference: &str) -> bool {
        self.0.lock().map(|session| session.contains_connection(reference)).unwrap_or(true)
    }
    pub fn connection_is_active(&self, reference: &str) -> bool {
        self.0.lock().map(|session| session.connection_is_active(reference)).unwrap_or(false)
    }
    pub(crate) fn skills(&self) -> Result<Vec<RuntimeSkill>, String> {
        let mut session = self.0.lock().map_err(|_| "plugin session unavailable")?;
        session.reconcile()?;
        Ok(session.runtimes.iter().filter(|runtime| !runtime.retired)
            .flat_map(|runtime| runtime.skills().iter().cloned()).collect())
    }
}

/// Conversation-owned instances, kept separate from the editable manual MCP
/// registry. A fresh installation read is required at discovery and dispatch.
pub struct PluginMcpSession {
    runtimes: Vec<PluginMcpRuntime>,
    read: InstallationReader,
}

impl PluginMcpSession {
    pub(crate) fn installed_revisions() -> Result<std::collections::BTreeMap<String, i64>, String> {
        use nola_application::{ApplicationCommand, PluginCommand};
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let mut result = std::collections::BTreeMap::new();
        let mut after = None;
        loop {
            let page: Vec<PluginInstallation> = app.execute(ApplicationCommand::Plugin {
                command: PluginCommand::List { after, limit: 128 },
            })?;
            let full = page.len() == 128;
            after = page.last().map(|record| record.manifest.id.clone());
            for record in page.into_iter().filter(|r| r.enabled && r.removed_at_ms.is_none()) {
                result.insert(record.manifest.id, record.revision);
            }
            if !full { break; }
        }
        Ok(result)
    }
    /// Load enabled plugins from canonical Application state. No process is
    /// started here. The shared owner pins both MCP resources and plugin Skills.
    pub(crate) fn persistent_default() -> Result<Option<Self>, String> {
        use crate::application_port::CliApplicationPort;
        use nola_application::{ApplicationCommand, PluginCommand};
        let mut app = CliApplicationPort::persistent_default()?;
        let mut after = None;
        let mut runtimes = Vec::new();
        loop {
            let page: Vec<PluginInstallation> = app.execute(ApplicationCommand::Plugin {
                command: PluginCommand::List { after, limit: 128 },
            })?;
            let full = page.len() == 128;
            after = page.last().map(|record| record.manifest.id.clone());
            for record in page.into_iter().filter(|record| record.enabled
                && record.removed_at_ms.is_none()) {
                let prepared = prepare_installed(&record)
                    .map_err(|error| format!("plugin {}: {error}", record.manifest.id))?;
                runtimes.push(PluginMcpRuntime::register(prepared)?);
            }
            if !full { break; }
        }
        if runtimes.is_empty() { return Ok(None); }
        Ok(Some(Self::new(runtimes, |id| {
            CliApplicationPort::persistent_default()?.execute(ApplicationCommand::Plugin {
                command: PluginCommand::Get { id: id.into() },
            })
        })))
    }

    pub fn new(
        runtimes: Vec<PluginMcpRuntime>,
        read: impl FnMut(&str) -> Result<Option<PluginInstallation>, String> + Send + 'static,
    ) -> Self {
        Self { runtimes, read: Box::new(read) }
    }

    pub fn reconcile(&mut self) -> Result<(), String> {
        for runtime in &mut self.runtimes {
            let current = match (self.read)(&runtime.owner().plugin_id) {
                Ok(current) => current,
                Err(_) => {
                    // An unavailable authority is not permission to use stale
                    // connections. Retire all instances before returning.
                    for runtime in &mut self.runtimes { runtime.retire(); }
                    return Err("plugin installation authority unavailable".into());
                }
            };
            runtime.reconcile(current.as_ref());
        }
        Ok(())
    }

    pub fn contains_connection(&self, reference: &str) -> bool {
        self.runtimes.iter().any(|runtime| runtime.prepared.host_servers().iter().chain(runtime.bound_configurations.iter())
            .any(|server| server["connection_ref"].as_str() == Some(reference)))
    }

    pub fn connection_is_active(&self, reference: &str) -> bool {
        self.runtimes.iter().any(|runtime| !runtime.retired && runtime.prepared.host_servers().iter().chain(runtime.bound_configurations.iter())
            .any(|server| server["connection_ref"].as_str() == Some(reference)))
    }

}

pub(crate) fn validate_restored_revisions(
    frozen: &std::collections::BTreeMap<String, i64>,
    current: &std::collections::BTreeMap<String, i64>,
) -> Result<(), String> {
    if frozen == current { Ok(()) } else {
        Err("Plugin installations changed since this Run started; do not replay pending effects against replacement plugins. Start a new Turn after resolving the previous Run.".into())
    }
}

pub(crate) fn prepare_installed(record: &PluginInstallation) -> Result<PluginRuntime, String> {
    nola_plugin_host::prepare_runtime(&crate::plugin_manager::package_store()?, record,
        |reference| {
            let vault = crate::host_credentials::HostCredentialVault::open(
                &crate::platform_paths::application_database_path()?)?;
            vault.plugin_record(&record.manifest.id, reference)?
                .filter(|credential| credential.identity == record.package_digest)
                .and_then(|credential| credential.secret)
                .ok_or_else(|| "plugin credential is not bound to this package".into())
        }, resolve_host_executable)
}

fn resolve_host_executable(name: &str) -> Result<std::path::PathBuf, String> {
    // The manifest validates a bare executable name. Ignore relative/empty
    // PATH entries: the user's current project must not shadow a dependency.
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
        let candidate = directory.join(name);
        if !candidate.is_file() { continue; }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if candidate.metadata().map_err(|_| "plugin dependency unavailable")?
                .permissions().mode() & 0o111 == 0 { continue; }
        }
        return candidate.canonicalize().map_err(|_| "plugin dependency unavailable".into());
    }
    Err("plugin dependency unavailable".into())
}

pub struct PluginMcpRuntime {
    prepared: PluginRuntime,
    retired: bool,
    bound_configurations: Vec<serde_json::Value>,
}

impl PluginMcpRuntime {
    /// Register metadata only. A separately admitted mcp_connect opens the
    /// connection. Caller retains this handle for its admitted lifetime.
    pub fn register(prepared: PluginRuntime) -> Result<Self, String> {
        let runtime = Self { prepared, retired: false,bound_configurations:Vec::new() };
        Ok(runtime)
    }

    pub fn owner(&self) -> &RuntimeOwner { self.prepared.owner() }
    pub fn skills(&self) -> &[RuntimeSkill] { self.prepared.skills() }

    /// `current` must be freshly read from Application, not a retained copy.
    /// A revision change revokes this exact instance, never retargets it.
    pub fn reconcile(&mut self, current: Option<&PluginInstallation>) -> bool {
        if self.retired { return false; }
        if !current.is_some_and(|record| self.prepared.matches_installation(record)) {
            self.retire();
            return false;
        }
        true
    }

    pub fn descriptions(&mut self, current: Option<&PluginInstallation>) -> Result<Vec<McpServerDescriptor>, String> {
        if !self.reconcile(current) { return Err("plugin runtime is retired".into()); }
        let values = if self.bound_configurations.is_empty() {self.prepared.host_servers()} else {&self.bound_configurations};
        let result = crate::mcp_host::describe_configured_servers(values)
            .map_err(|error| error.code.to_string())
            .and_then(|values| values.into_iter().map(|value|
                serde_json::from_value(value).map_err(|_| "invalid plugin MCP descriptor".to_string())
            ).collect());
        if result.is_err() { self.retire(); }
        result
    }

    /// Synchronous drain: worker-only. Existing requests retain their transport
    /// deadline; new requests fail lookup. Child processes are killed/reaped
    /// before the pinned package is dropped. Does not erase manual MCP servers.
    pub fn retire(&mut self) {
        if self.retired { return; }
        self.retired = true;
        let mut draining = Vec::new();
        for server in &self.bound_configurations {
            let transport = if crate::mcp_stdio::is_stdio_configuration(server) {
                McpTransportKind::Stdio
            } else { McpTransportKind::StreamableHttp };
            // Prepared configurations always carry a unique instance reference.
            if let Some(reference) = server["connection_ref"].as_str() {
                if let Ok(retirement) = crate::mcp_host::revoke_connection(transport, reference) {
                    draining.push(retirement);
                }
            }
        }
        // Revoke every owned entry before waiting for any in-flight request.
        drop(draining);
    }
}

impl Drop for PluginMcpRuntime {
    fn drop(&mut self) { self.retire(); }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn plugin_cold_restore_requires_the_exact_installation_revisions() {
        let frozen = std::collections::BTreeMap::from([("example".into(), 2)]);
        assert!(validate_restored_revisions(&frozen, &frozen).is_ok());
        assert!(validate_restored_revisions(&frozen, &Default::default()).is_err());
        assert!(validate_restored_revisions(&frozen,
            &std::collections::BTreeMap::from([("example".into(), 4)])).is_err());
        assert!(validate_restored_revisions(&Default::default(), &frozen).is_err());
    }

    #[test]
    fn plugin_skill_catalog_is_namespaced_pinned_and_revocable() {
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("skill")).unwrap();
        fs::write(source.path().join("skill/SKILL.md"), "---\nname: shared-title\ndescription: Pinned plugin instructions\n---\n\nPINNED\n").unwrap();
        fs::write(source.path().join("nola-plugin.json"), json!({
            "schema_version":1,"id":"skill-fixture","version":"1","display_name":"Skill fixture","description":"",
            "skills":[{"id":"owned","path":"./skill"}]
        }).to_string()).unwrap();
        let package = nola_plugin_host::inspect(source.path()).unwrap();
        let store = tempfile::tempdir().unwrap();
        package.publish(store.path()).unwrap();
        let record = PluginInstallation { manifest:package.manifest().clone(), package_digest:package.digest().into(),
            enabled:true, revision:2, installed_at_ms:0, updated_at_ms:1, removed_at_ms:None };
        let runtime = nola_plugin_host::prepare_runtime(store.path(), &record, |_| panic!(), |_| panic!()).unwrap();
        let current = std::sync::Arc::new(std::sync::Mutex::new(Some(record)));
        let reader = current.clone();
        let shared: SharedPluginSession = PluginMcpSession::new(vec![PluginMcpRuntime::register(runtime).unwrap()],
            move |_| Ok(reader.lock().unwrap().clone())).into();
        let empty_root = tempfile::tempdir().unwrap();
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![empty_root.path().into()]))
            .with_plugins(shared.clone());
        let index = skills.index().unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0]["id"], "plugin:skill-fixture:skill:owned");
        assert_eq!(skills.invocation_schema()["enum"], json!(["plugin:skill-fixture:skill:owned"]));
        fs::write(source.path().join("skill/SKILL.md"), "MUTATED").unwrap();
        let directory = skills.plugin_skills().unwrap()[0].directory.clone();
        assert!(fs::read_to_string(directory.join("SKILL.md")).unwrap().contains("PINNED"));
        let disabled = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![])).with_plugins(shared);
        assert!(disabled.index().unwrap().is_empty());
        *current.lock().unwrap() = None;
        assert!(skills.index().unwrap().is_empty());
        assert!(skills.plugin_skills().unwrap().is_empty());
    }

    #[test]
    fn plugin_mcp_real_process_lifecycle_preserves_other_instances() {
        let source = tempfile::tempdir().unwrap();
        let script = r#"#!/usr/bin/env python3
import json, os, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method', '')
    if method == 'notifications/initialized':
        continue
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'echo','description':'Echo arguments','inputSchema':{'type':'object'}}]}
    elif method == 'tools/call':
        result = {'content':[{'type':'text','text':json.dumps({'pid':os.getpid(),'arguments':request['params']['arguments']})}]}
    else:
        print(json.dumps({'jsonrpc':'2.0','id':request['id'],'error':{'code':-32601,'message':'Method not found'}}), flush=True)
        continue
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#;
        fs::write(source.path().join("server.py"), script).unwrap();
        fs::set_permissions(source.path().join("server.py"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(source.path().join("nola-plugin.json"), json!({
            "schema_version":1,"id":"lifecycle","version":"1","display_name":"Lifecycle","description":"",
            "mcp_servers":[{"id":"echo","transport":{"type":"stdio",
                "executable":{"type":"package","path":"./server.py"}}}]
        }).to_string()).unwrap();
        let package = nola_plugin_host::inspect(source.path()).unwrap();
        let store = tempfile::tempdir().unwrap();
        package.publish(store.path()).unwrap();
        let mut record = PluginInstallation { manifest:package.manifest().clone(), package_digest:package.digest().into(),
            enabled:true, revision:2, installed_at_ms:0, updated_at_ms:1, removed_at_ms:None };
        let prepare = |record: &PluginInstallation| nola_plugin_host::prepare_runtime(store.path(), record,
            |_| panic!("no credentials"), |_| panic!("packaged executable")).unwrap();
        let mut first = PluginMcpRuntime::register(prepare(&record)).unwrap();
        let mut second = PluginMcpRuntime::register(prepare(&record)).unwrap();
        let initial = first.descriptions(Some(&record)).unwrap();
        assert_eq!(initial[0].availability,nola_core::agent::capability::McpServerAvailability::AuthorizationRequired);
        assert!(initial[0].tools.is_empty());
        let connect = |runtime:&mut PluginMcpRuntime| {
            let values = crate::mcp_connection_authority::bind_configurations(runtime.prepared.host_servers(),
                "plugin-fixture",&uuid::Uuid::new_v4().to_string()).unwrap();
            crate::mcp_host::register_configured_servers(&values).unwrap();
            for value in &values {
                let reference = value["connection_ref"].as_str().unwrap();
                let expected = crate::mcp_connection_authority::permission_resource(reference).unwrap();
                crate::mcp_host::connect_approved(reference,&expected).unwrap();
            }
            runtime.bound_configurations = values;
            crate::mcp_host::describe_connected_cache(&runtime.bound_configurations).unwrap().into_iter()
                .map(|value|serde_json::from_value::<McpServerDescriptor>(value).unwrap()).collect::<Vec<_>>()
        };
        let servers = connect(&mut first);
        let others = connect(&mut second);
        assert_eq!(servers.len(), 1);
        assert!(servers[0].tools.contains_key("echo"));
        assert_ne!(servers[0].connection_ref, others[0].connection_ref);
        let current = std::sync::Arc::new(std::sync::Mutex::new(Ok(Some(record.clone()))));
        let reader = current.clone();
        let mut session = PluginMcpSession::new(vec![first], move |_| reader.lock().unwrap().clone());
        assert!(session.contains_connection(&servers[0].connection_ref));
        assert!(session.connection_is_active(&servers[0].connection_ref));
        assert!(!session.contains_connection(&others[0].connection_ref));
        assert_eq!(session.runtimes.iter().filter(|runtime|!runtime.retired).count(),1);
        let call = |reference: &str| crate::mcp_host::call_tool(
            McpTransportKind::Stdio, reference, "echo", json!({"value":"你好"}));
        let output = call(&servers[0].connection_ref).unwrap();
        assert!(!output.is_error);
        let result: serde_json::Value = serde_json::from_str(output.output["content"][0]["text"].as_str().unwrap()).unwrap();
        let child_pid = result["pid"].as_i64().unwrap() as libc::pid_t;
        assert_eq!(result["arguments"]["value"], "你好");
        record.enabled = false; record.revision += 1;
        *current.lock().unwrap() = Ok(Some(record.clone()));
        session.reconcile().unwrap();
        assert!(!session.connection_is_active(&servers[0].connection_ref));
        // Signal zero probes only the child created by this isolated fixture.
        assert_eq!(unsafe { libc::kill(child_pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        assert_eq!(call(&servers[0].connection_ref).unwrap_err().code, "mcp_connection_unavailable");
        assert!(!call(&others[0].connection_ref).unwrap().is_error);
        assert!(session.runtimes.iter().all(|runtime|runtime.retired));
        // Authority read failure must also retire connections, not leave the
        // previously discovered instance usable under a stale enabled record.
        let mut unavailable = PluginMcpSession::new(vec![second], |_| Err("private backend detail".into()));
        assert_eq!(unavailable.reconcile().unwrap_err(), "plugin installation authority unavailable");
        assert_eq!(call(&others[0].connection_ref).unwrap_err().code, "mcp_connection_unavailable");
        drop(unavailable);
        drop(session);
        assert_eq!(fs::read_dir(store.path()).unwrap().count(), 1, "only installed package remains");
    }
}
