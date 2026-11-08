// SPDX-License-Identifier: Apache-2.0
//! Host-only, pinned resource projection. Preparation neither starts MCP nor
//! grants permission. The owner must stop its connections before dropping this
//! handle. Never serialize resolved server configuration into product storage.

use crate::verify_installed;
use nola_domain::plugin::{PluginExecutable, PluginInstallation, PluginMcpTransport};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::{Path, PathBuf}};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwner {
    pub plugin_id: String,
    pub revision: i64,
    pub package_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSkill {
    pub resource_id: String,
    pub directory: PathBuf,
}

// Deliberately no Debug/Serialize: server configuration can contain secrets.
pub struct PluginRuntime {
    owner: RuntimeOwner,
    skills: Vec<RuntimeSkill>,
    servers: Vec<Value>,
    _snapshot: tempfile::TempDir,
}

impl PluginRuntime {
    pub fn owner(&self) -> &RuntimeOwner { &self.owner }
    pub fn skills(&self) -> &[RuntimeSkill] { &self.skills }
    /// For the existing Host MCP transport only, not model or client payloads.
    pub fn host_servers(&self) -> &[Value] { &self.servers }
    /// Refresh against authoritative installation state before admitting new
    /// calls. An old owner must never be redirected to a replacement package.
    pub fn matches_installation(&self, record: &PluginInstallation) -> bool {
        record.enabled && record.removed_at_ms.is_none()
            && record.manifest.id == self.owner.plugin_id
            && record.revision == self.owner.revision
            && record.package_digest == self.owner.package_digest
    }
}

pub fn prepare_runtime(
    package_store: &Path,
    record: &PluginInstallation,
    mut resolve_secret: impl FnMut(&str) -> Result<String, String>,
    mut resolve_executable: impl FnMut(&str) -> Result<PathBuf, String>,
) -> Result<PluginRuntime, String> {
    if !record.enabled || record.removed_at_ms.is_some() || record.revision <= 0 {
        return Err("plugin is not enabled".into());
    }
    let package = verify_installed(package_store, &record.package_digest)?;
    if package.manifest() != &record.manifest {
        return Err("plugin manifest does not match installation".into());
    }
    // Reuse captured bytes, never re-read the source after digest validation.
    // This is lifetime isolation from installation replacement, not an OS sandbox.
    let snapshot = package.stage(package_store)?;
    let root = snapshot.path().canonicalize().map_err(|e| e.to_string())?;
    let manifest = package.manifest();
    let owner = RuntimeOwner { plugin_id: manifest.id.clone(), revision: record.revision,
        package_digest: record.package_digest.clone() };
    let skills = manifest.skills.iter().map(|skill| Ok(RuntimeSkill {
        resource_id: manifest.resource_id("skill", &skill.id)?,
        directory: root.join(skill.path.strip_prefix("./").unwrap()),
    })).collect::<Result<Vec<_>, String>>()?;
    let mut servers = Vec::new();
    for server in &manifest.mcp_servers {
        let id = manifest.resource_id("mcp", &server.id)?;
        let mut value = json!({"server_id":id, "name":format!("{} / {}", manifest.display_name, server.id),
            "enabled":true, "revision":record.revision,
            "connection_ref":format!("{}:revision:{}:{}:{}", id, record.revision, record.package_digest,
                snapshot.path().file_name().unwrap().to_string_lossy())});
        match &server.transport {
            PluginMcpTransport::Stdio { executable, args, secret_env } => {
                let command = match executable {
                    PluginExecutable::Package { path } => root.join(path.strip_prefix("./").unwrap()),
                    PluginExecutable::Host { name } => {
                        // The resolver performs host dependency admission. Do not
                        // leave PATH resolution to the child process at spawn.
                        let path = resolve_executable(name).map_err(|_| "plugin host executable unavailable")?;
                        if !path.is_absolute() || !path.is_file() {
                            return Err("plugin host executable must resolve to an absolute file".into());
                        }
                        path
                    }
                };
                let mut env = BTreeMap::new();
                for (name, reference) in secret_env {
                    let secret = resolve_secret(reference).map_err(|_| "plugin credential unavailable")?;
                    if secret.is_empty() || secret.len() > 16_384 || secret.contains('\0') {
                        return Err("invalid plugin environment credential".into());
                    }
                    env.insert(name, secret);
                }
                value["type"] = json!("stdio");
                value["command"] = json!(command.to_str().ok_or("plugin executable path must be UTF-8")?);
                value["cwd"] = json!(root.to_str().ok_or("plugin package path must be UTF-8")?);
                value["args"] = json!(args);
                value["env"] = json!(env);
            }
            PluginMcpTransport::StreamableHttp { url, bearer_secret_ref } => {
                value["type"] = json!("streamable_http");
                value["endpoint"] = json!(url);
                if let Some(reference) = bearer_secret_ref {
                    let secret = resolve_secret(reference).map_err(|_| "plugin credential unavailable")?;
                    if secret.is_empty() || secret.len() > 16_384
                        || !secret.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
                        return Err("invalid plugin bearer credential".into());
                    }
                    value["headers"] = json!({"Authorization":format!("Bearer {secret}")});
                }
            }
        }
        servers.push(value);
    }
    Ok(PluginRuntime { owner, skills, servers, _snapshot: snapshot })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, PluginInstallation) {
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("research")).unwrap();
        fs::write(source.path().join("research/SKILL.md"), "# Research\nOriginal content").unwrap();
        fs::write(source.path().join("nola-plugin.json"), json!({
            "schema_version":1, "id":"example", "version":"1", "display_name":"Example", "description":"",
            "skills":[{"id":"research","path":"./research"}],
            "mcp_servers":[{"id":"remote","transport":{"type":"streamable_http",
                "url":"http://127.0.0.1:9191/mcp","bearer_secret_ref":"plugin/example/token"}}]
        }).to_string()).unwrap();
        let package = crate::inspect(source.path()).unwrap();
        let store = tempfile::tempdir().unwrap();
        package.publish(store.path()).unwrap();
        let record = PluginInstallation { manifest: package.manifest().clone(), package_digest:package.digest().into(),
            enabled:true, revision:2, installed_at_ms:0, updated_at_ms:1, removed_at_ms:None };
        (store, record)
    }

    #[test]
    fn runtime_pins_bytes_namespaces_resources_and_cleans_up() {
        let (store, record) = fixture();
        let prepare = || prepare_runtime(store.path(), &record, |reference| {
            assert_eq!(reference, "plugin/example/token"); Ok("test-token".into())
        }, |_| panic!("HTTP plugin does not resolve executables")).unwrap();
        let runtime = prepare();
        let second = prepare();
        assert_ne!(runtime.host_servers()[0]["connection_ref"], second.host_servers()[0]["connection_ref"]);
        assert_eq!(runtime.host_servers()[0]["server_id"], "plugin:example:mcp:remote");
        assert_eq!(runtime.host_servers()[0]["headers"]["Authorization"], "Bearer test-token");
        assert_eq!(runtime.skills()[0].resource_id, "plugin:example:skill:research");
        let path = runtime.skills()[0].directory.join("SKILL.md");
        fs::write(store.path().join(&record.package_digest).join("research/SKILL.md"), "changed").unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("Original"));
        assert!(runtime.matches_installation(&record));
        let mut disabled = record.clone(); disabled.enabled = false;
        assert!(!runtime.matches_installation(&disabled));
        disabled.enabled = true; disabled.revision += 1;
        assert!(!runtime.matches_installation(&disabled));
        drop(runtime);
        assert!(!path.exists());
        assert!(second.skills()[0].directory.exists());
    }

    #[test]
    fn disabled_removed_and_tampered_packages_never_resolve_credentials() {
        let (store, mut record) = fixture();
        record.enabled = false;
        assert!(prepare_runtime(store.path(), &record, |_| panic!("disabled"), |_| panic!("disabled")).is_err());
        record.enabled = true; record.removed_at_ms = Some(2);
        assert!(prepare_runtime(store.path(), &record, |_| panic!("removed"), |_| panic!("removed")).is_err());
        record.removed_at_ms = None;
        fs::write(store.path().join(&record.package_digest).join("research/SKILL.md"), "changed").unwrap();
        assert!(prepare_runtime(store.path(), &record, |_| panic!("tampered"), |_| panic!("tampered")).is_err());
    }

    #[test]
    fn credential_failure_is_redacted_and_failed_snapshot_is_cleaned() {
        let (store, record) = fixture();
        let before = fs::read_dir(store.path()).unwrap().count();
        let error = prepare_runtime(store.path(), &record, |_| Err("sensitive-provider-detail".into()),
            |_| panic!("HTTP")).err().unwrap();
        assert_eq!(error, "plugin credential unavailable");
        assert_eq!(fs::read_dir(store.path()).unwrap().count(), before);
        assert!(prepare_runtime(store.path(), &record, |_| Ok("token\r\nInjected: true".into()),
            |_| panic!("HTTP")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn stdio_projection_uses_pinned_argv_and_never_executes_during_preparation() {
        use std::os::unix::fs::PermissionsExt;
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("server.sh"), "#!/bin/sh\ntouch EXECUTED\n").unwrap();
        fs::set_permissions(source.path().join("server.sh"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(source.path().join("nola-plugin.json"), json!({
            "schema_version":1, "id":"local", "version":"1", "display_name":"Local", "description":"",
            "mcp_servers":[{"id":"server","transport":{"type":"stdio",
                "executable":{"type":"package","path":"./server.sh"},
                "args":["a b","$(never-execute)"],"secret_env":{"TOKEN":"local/token"}}}]
        }).to_string()).unwrap();
        let package = crate::inspect(source.path()).unwrap();
        let store = tempfile::tempdir().unwrap();
        package.publish(store.path()).unwrap();
        let record = PluginInstallation { manifest:package.manifest().clone(), package_digest:package.digest().into(),
            enabled:true, revision:2, installed_at_ms:0, updated_at_ms:1, removed_at_ms:None };
        let runtime = prepare_runtime(store.path(), &record, |_| Ok("test-token".into()),
            |_| panic!("packaged executable must not resolve PATH")).unwrap();
        let config = &runtime.host_servers()[0];
        let cwd = Path::new(config["cwd"].as_str().unwrap());
        assert!(!cwd.join("EXECUTED").exists());
        assert_eq!(Path::new(config["command"].as_str().unwrap()), cwd.join("server.sh"));
        assert_eq!(config["args"], json!(["a b","$(never-execute)"]));
        assert_eq!(config["env"]["TOKEN"], "test-token");
        assert!(!serde_json::to_string(&record).unwrap().contains("test-token"));
    }
}
