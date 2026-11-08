// SPDX-License-Identifier: Apache-2.0
//! Declarative package input. Validation is not installation or authorization.
//! Host validates real paths/digests; Application owns activation and persistence.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_MANIFEST_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallation {
    pub manifest: PluginManifest,
    /// Content-addressed package identity; local paths are resolved by Host.
    pub package_digest: String,
    pub enabled: bool,
    pub revision: i64,
    pub installed_at_ms: i64,
    pub updated_at_ms: i64,
    pub removed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub skills: Vec<PluginSkill>,
    #[serde(default)]
    pub mcp_servers: Vec<PluginMcpServer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSkill {
    pub id: String,
    /// Package-relative directory containing SKILL.md, never a host path.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMcpServer {
    pub id: String,
    pub transport: PluginMcpTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginMcpTransport {
    Stdio {
        executable: PluginExecutable,
        #[serde(default)]
        args: Vec<String>,
        /// Environment variable -> credential-store reference, never a secret.
        #[serde(default)]
        secret_env: BTreeMap<String, String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        bearer_secret_ref: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginExecutable {
    Package { path: String },
    /// Explicit host dependency resolved and admitted by the Host at activation.
    Host { name: String },
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128
        && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub fn validate_package_path(path: &str) -> Result<(), String> {
    let relative = path.strip_prefix("./").ok_or("plugin paths must start with ./")?;
    if relative.is_empty() || path.len() > 1024
        || relative.contains(['\\', ':']) || relative.chars().any(char::is_control)
        || relative.split('/').any(|component| matches!(component, "" | "." | "..")) {
        return Err("plugin path must stay within its package".into());
    }
    Ok(())
}

fn secret_reference(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_/.:".contains(&b))
}

impl PluginManifest {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > PLUGIN_MANIFEST_MAX_BYTES {
            return Err("plugin manifest exceeds byte budget".into());
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|e| format!("invalid plugin manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || !identifier(&self.id) {
            return Err("unsupported plugin schema or invalid plugin id".into());
        }
        if self.version.is_empty() || self.version.len() > 128
            || !self.version.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b))
            || self.display_name.trim().is_empty() || self.display_name.len() > 256
            || self.display_name.chars().any(char::is_control) || self.description.len() > 8192 {
            return Err("invalid plugin presentation/version metadata".into());
        }
        if self.skills.len() + self.mcp_servers.len() == 0
            || self.skills.len() + self.mcp_servers.len() > 128 {
            return Err("plugin must contribute 1..128 resources".into());
        }
        let mut ids = BTreeSet::new();
        for skill in &self.skills {
            if !identifier(&skill.id) || !ids.insert(("skill", &skill.id)) {
                return Err("invalid or duplicate plugin Skill id".into());
            }
            validate_package_path(&skill.path)?;
        }
        for server in &self.mcp_servers {
            if !identifier(&server.id) || !ids.insert(("mcp", &server.id)) {
                return Err("invalid or duplicate plugin MCP id".into());
            }
            match &server.transport {
                PluginMcpTransport::Stdio { executable, args, secret_env } => {
                    match executable {
                        PluginExecutable::Package { path } => validate_package_path(path)?,
                        PluginExecutable::Host { name } => {
                            if name.is_empty() || name.len() > 128 || name.starts_with('.')
                                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
                                return Err("invalid plugin host executable name".into());
                            }
                        }
                    }
                    if args.len() > 128 || args.iter().any(|arg| arg.len() > 8192 || arg.contains('\0'))
                        || secret_env.len() > 64 || secret_env.iter().any(|(name, reference)| {
                            name.is_empty() || name.len() > 128
                                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                                || name.as_bytes()[0].is_ascii_digit() || !secret_reference(reference)
                        }) {
                        return Err("invalid plugin arguments or credential references".into());
                    }
                }
                PluginMcpTransport::StreamableHttp { url, bearer_secret_ref } => {
                    let parsed = url::Url::parse(url).map_err(|_| "invalid plugin MCP URL")?;
                    if url.len() > 8192 || !matches!(parsed.scheme(), "http" | "https")
                        || parsed.host_str().is_none() || !parsed.username().is_empty()
                        || parsed.password().is_some() || parsed.fragment().is_some()
                        || bearer_secret_ref.as_ref().is_some_and(|r| !secret_reference(r)) {
                        return Err("invalid plugin MCP endpoint or credential reference".into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Typed namespaces prevent a package resource from replacing a built-in.
    pub fn resource_id(&self, kind: &str, local_id: &str) -> Result<String, String> {
        self.validate()?;
        let found = match kind {
            "skill" => self.skills.iter().any(|s| s.id == local_id),
            "mcp" => self.mcp_servers.iter().any(|s| s.id == local_id),
            _ => false,
        };
        if !found { return Err("undeclared plugin resource".into()); }
        Ok(format!("plugin:{}:{kind}:{local_id}", self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> PluginManifest {
        PluginManifest { schema_version: 1, id: "example".into(), version: "1.0.0".into(),
            display_name: "Example".into(), description: "Fixture".into(),
            skills: vec![PluginSkill { id: "research".into(), path: "./skills/research".into() }],
            mcp_servers: vec![PluginMcpServer { id: "tools".into(), transport: PluginMcpTransport::Stdio {
                executable: PluginExecutable::Package { path: "./bin/server".into() },
                args: vec![], secret_env: BTreeMap::new(),
            }}] }
    }
    #[test]
    fn roundtrip_and_namespaced_resources() {
        let value = manifest();
        assert_eq!(PluginManifest::decode(&serde_json::to_vec(&value).unwrap()).unwrap(), value);
        assert_eq!(value.resource_id("skill", "research").unwrap(), "plugin:example:skill:research");
        assert!(value.resource_id("mcp", "research").is_err());
    }
    #[test]
    fn rejects_path_escapes_before_host_resolution() {
        for path in ["/tmp/a", "../a", "./../a", "./a/../../b", "./a//b", "./C:/a", "./a\\b", "./", "./a/./b", "./a\0b"] {
            assert!(validate_package_path(path).is_err(), "{path:?}");
        }
        assert!(validate_package_path("./skills/研究").is_ok());
    }
    #[test]
    fn rejects_duplicate_resources_and_future_schemas() {
        let mut value = manifest();
        value.skills.push(value.skills[0].clone());
        assert!(value.validate().is_err());
        let mut value = manifest(); value.schema_version = 2;
        assert!(value.validate().is_err());
    }
    #[test]
    fn rejects_inline_endpoint_credentials() {
        let mut value = manifest();
        for url in ["https://user:password@example.com/mcp", "file:///tmp/mcp", "https://example.com/#secret"] {
            value.mcp_servers[0].transport = PluginMcpTransport::StreamableHttp { url: url.into(), bearer_secret_ref: None };
            assert!(value.validate().is_err());
        }
        value.mcp_servers[0].transport = PluginMcpTransport::StreamableHttp {
            url: "http://192.168.31.18:8081/mcp".into(), bearer_secret_ref: Some("credential/example".into()) };
        assert!(value.validate().is_ok()); // Network authorization still belongs to Host/Core.
    }
    #[test]
    fn rejects_unknown_fields_and_oversized_input() {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["enabled"] = true.into();
        assert!(PluginManifest::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(PluginManifest::decode(&vec![b' '; PLUGIN_MANIFEST_MAX_BYTES + 1]).is_err());
    }
}
