// SPDX-License-Identifier: Apache-2.0
//! MCP server product use-case boundary.

use nola_store_port::mcp::{McpRepository, StoredMcpServerRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub configuration_json: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpCommand {
    ApplyChanges {
        commands: Vec<McpCommand>,
    },
    Create {
        id: String,
        display_name: String,
        enabled: bool,
        configuration_json: String,
        created_at_ms: i64,
    },
    Upsert {
        input: McpServer,
    },
    Configure {
        id: String,
        expected_revision: i64,
        display_name: String,
        enabled: bool,
        configuration_json: String,
        updated_at_ms: i64,
    },
    DeleteChecked {
        id: String,
        expected_revision: i64,
        deleted_at_ms: i64,
    },
    List,
    Delete {
        id: String,
        deleted_at_ms: i64,
    },
}

pub(crate) fn execute_mcp_command(
    store: &mut impl McpRepository,
    command: McpCommand,
) -> Result<Value, String> {
    match command {
        McpCommand::ApplyChanges { commands } => {
            if !store.has_write_unit_of_work() {
                return Err("MCP changes require durable command admission".into());
            }
            if commands.len() > 128 {
                return Err("MCP changes are limited to 128 operations".into());
            }
            if commands.iter().any(|command| {
                !matches!(
                    command,
                    McpCommand::Create { .. }
                        | McpCommand::Configure { .. }
                        | McpCommand::DeleteChecked { .. }
                )
            }) {
                return Err("MCP changes only accept create, configure, and delete_checked".into());
            }
            commands
                .into_iter()
                .map(|command| execute_mcp_command(store, command))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        McpCommand::Create {
            id,
            display_name,
            enabled,
            configuration_json,
            created_at_ms,
        } => {
            if !store.has_write_unit_of_work() {
                return Err("MCP creation requires durable command admission".into());
            }
            if store
                .mcp_server_identity_exists(&id)
                .map_err(|error| error.to_string())?
            {
                return Err(format!(
                    "mcp_identity_conflict: server identity already exists: {id}"
                ));
            }
            let saved = store
                .upsert_mcp_server(&StoredMcpServerRecord {
                    id,
                    display_name,
                    enabled,
                    configuration_json,
                    revision: 0,
                    created_at_ms,
                    updated_at_ms: created_at_ms,
                    deleted_at_ms: None,
                })
                .map_err(|error| format!("create MCP server failed: {error}"))?;
            serde_json::to_value(McpServer::from(saved))
                .map_err(|error| format!("encode MCP server result failed: {error}"))
        }
        McpCommand::Configure {
            id,
            expected_revision,
            display_name,
            enabled,
            configuration_json,
            updated_at_ms,
        } => {
            let mut current = editable_server(store, &id, expected_revision)?;
            current.display_name = display_name;
            current.enabled = enabled;
            current.configuration_json = configuration_json;
            current.updated_at_ms = updated_at_ms;
            let saved = store
                .upsert_mcp_server(&current)
                .map_err(|error| format!("configure MCP server failed: {error}"))?;
            serde_json::to_value(McpServer::from(saved))
                .map_err(|error| format!("encode MCP server result failed: {error}"))
        }
        McpCommand::DeleteChecked {
            id,
            expected_revision,
            deleted_at_ms,
        } => {
            editable_server(store, &id, expected_revision)?;
            Ok(json!(store.delete_mcp_server(&id, deleted_at_ms).map_err(
                |error| format!("delete MCP server failed: {error}")
            )?))
        }
        McpCommand::Upsert { input } => {
            let stored = StoredMcpServerRecord::from(input);
            let saved = store
                .upsert_mcp_server(&stored)
                .map_err(|error| format!("save MCP server failed: {error}"))?;
            serde_json::to_value(McpServer::from(saved))
                .map_err(|error| format!("encode MCP server result failed: {error}"))
        }
        McpCommand::List => {
            let servers: Vec<McpServer> = store
                .list_mcp_servers()
                .map_err(|error| format!("load MCP servers failed: {error}"))?
                .into_iter()
                .map(McpServer::from)
                .collect();
            serde_json::to_value(servers)
                .map_err(|error| format!("encode MCP server list failed: {error}"))
        }
        McpCommand::Delete { id, deleted_at_ms } => Ok(json!(store
            .delete_mcp_server(&id, deleted_at_ms)
            .map_err(|error| format!("delete MCP server failed: {error}"))?)),
    }
}

fn editable_server(
    store: &impl McpRepository,
    id: &str,
    expected_revision: i64,
) -> Result<StoredMcpServerRecord, String> {
    if !store.has_write_unit_of_work() {
        return Err("MCP editing requires durable command admission".into());
    }
    if expected_revision < 1 {
        return Err("MCP editing requires a positive expected revision".into());
    }
    let current = store
        .mcp_server(id)
        .map_err(|error| format!("load MCP server failed: {error}"))?
        .ok_or_else(|| format!("MCP server is missing or deleted: {id}"))?;
    if current.revision != expected_revision {
        return Err(format!(
            "mcp_revision_conflict: {id}: expected {expected_revision}, actual {}",
            current.revision
        ));
    }
    Ok(current)
}

impl From<StoredMcpServerRecord> for McpServer {
    fn from(value: StoredMcpServerRecord) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            enabled: value.enabled,
            configuration_json: value.configuration_json,
            revision: value.revision,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

impl From<McpServer> for StoredMcpServerRecord {
    fn from(value: McpServer) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            enabled: value.enabled,
            configuration_json: value.configuration_json,
            revision: value.revision,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

#[cfg(test)]
#[path = "mcp_port_tests.rs"]
mod port_tests;
