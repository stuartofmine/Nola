use crate::application_port::CliApplicationPort;
use nola_application::{ApplicationCommand, McpCommand, McpServer};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn load_web_mcp_servers() -> Result<Vec<Value>, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    let records: Vec<McpServer> = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::List,
    })?;
    records
        .into_iter()
        .map(|record| {
            let mut value = serde_json::from_str::<Value>(&record.configuration_json)
                .map_err(|error| format!("decode MCP server {} failed: {error}", record.id))?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| format!("MCP server {} is not a JSON object", record.id))?;
            object.insert("server_id".to_string(), Value::String(record.id));
            object.insert("name".to_string(), Value::String(record.display_name));
            object.insert("enabled".to_string(), Value::Bool(record.enabled));
            object.insert("revision".to_string(), Value::from(record.revision));
            Ok(value)
        })
        .collect()
}

pub fn save_web_mcp_changes(expected: &[Value], servers: &[Value]) -> Result<(), String> {
    let command = mcp_changes_command(expected, servers)?;
    let mut application = CliApplicationPort::persistent_default()?;
    let _: Value = application.execute(ApplicationCommand::Mcp { command })?;
    Ok(())
}

fn mcp_changes_command(expected: &[Value], servers: &[Value]) -> Result<McpCommand, String> {
    validate_mcp_servers(expected)?;
    validate_mcp_servers(servers)?;
    let mut revisions = HashMap::new();
    for (index, value) in expected.iter().enumerate() {
        let id = mcp_server_id(value, index)?;
        let revision = value
            .get("revision")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("MCP baseline requires a positive revision: {id}"))?;
        revisions.insert(id, revision);
    }
    let now = unix_time_ms()?;
    let mut commands = Vec::new();
    for (index, server) in servers.iter().enumerate() {
        let id = mcp_server_id(server, index)?;
        let display_name = server
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&id)
            .to_string();
        let enabled = server
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut configuration = server.clone();
        configuration.as_object_mut().unwrap().remove("revision");
        let configuration_json =
            serde_json::to_string(&configuration).map_err(|error| error.to_string())?;
        commands.push(match revisions.remove(&id) {
            Some(expected_revision) => McpCommand::Configure {
                id,
                expected_revision,
                display_name,
                enabled,
                configuration_json,
                updated_at_ms: now,
            },
            None => McpCommand::Create {
                id,
                display_name,
                enabled,
                configuration_json,
                created_at_ms: now,
            },
        });
    }
    let mut deleted = revisions.into_iter().collect::<Vec<_>>();
    deleted.sort_by(|left, right| left.0.cmp(&right.0));
    commands.extend(
        deleted
            .into_iter()
            .map(|(id, expected_revision)| McpCommand::DeleteChecked {
                id,
                expected_revision,
                deleted_at_ms: now,
            }),
    );
    if commands.len() > 128 {
        return Err("MCP changes are limited to 128 operations".into());
    }
    Ok(McpCommand::ApplyChanges { commands })
}

fn mcp_server_id(value: &Value, index: usize) -> Result<String, String> {
    value
        .get("server_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("MCP server at index {index} has no stable ID"))
}

fn validate_mcp_servers(values: &[Value]) -> Result<(), String> {
    if values.len() > 128 {
        return Err("MCP servers are limited to 128 entries".to_string());
    }
    let mut ids = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        if !value.is_object() {
            return Err(format!("MCP server at index {index} must be an object"));
        }
        let id = mcp_server_id(value, index)?;
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate MCP server ID: {id}"));
        }
        if id.chars().count() > 256 {
            return Err(format!(
                "MCP server ID at index {index} exceeds 256 characters"
            ));
        }
    }
    Ok(())
}

fn unix_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read system clock failed: {error}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| "system clock exceeds i64".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_application::{ApplicationCommandReceiptOutcome, ExecuteReceivedCommand};
    use nola_application_bootstrap::ApplicationRuntime;
    use serde_json::json;

    fn submit(runtime: &mut ApplicationRuntime, request_id: &str, command: McpCommand) -> bool {
        let command = ApplicationCommand::Mcp { command };
        let command_json = serde_json::to_string(&command).unwrap();
        let receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "web-mcp-test".into(),
                    request_id: request_id.into(),
                    command_type: "mcp".into(),
                    command_json,
                    expected_revision: None,
                    received_at: "2026-09-06T00:00:00Z".into(),
                },
                command,
            )
            .unwrap();
        matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        )
    }

    #[test]
    fn browser_baseline_never_deletes_another_editors_addition() {
        let root = std::env::temp_dir().join(format!("nola-web-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("product.db");
        let mut runtime = nola_application_bootstrap::open(&database).unwrap();
        assert!(submit(
            &mut runtime,
            "create",
            mcp_changes_command(
                &[],
                &[json!({
                    "server_id": "first", "name": "First", "enabled": false
                })]
            )
            .unwrap()
        ));
        assert!(submit(
            &mut runtime,
            "another-editor",
            mcp_changes_command(
                &[],
                &[json!({
                    "server_id": "second", "name": "Second", "enabled": false
                })]
            )
            .unwrap()
        ));
        let baseline = [json!({"server_id":"first", "revision":1})];
        assert!(submit(
            &mut runtime,
            "delete-first",
            mcp_changes_command(&baseline, &[]).unwrap()
        ));
        let remaining = runtime
            .execute(ApplicationCommand::Mcp {
                command: McpCommand::List,
            })
            .unwrap();
        assert_eq!(remaining.as_array().unwrap().len(), 1);
        assert_eq!(remaining[0]["id"], "second");
        assert!(!submit(
            &mut runtime,
            "stale",
            mcp_changes_command(
                &baseline,
                &[json!({
                    "server_id":"first", "name":"Stale", "enabled":false
                })]
            )
            .unwrap()
        ));
        assert_eq!(
            remaining,
            runtime
                .execute(ApplicationCommand::Mcp {
                    command: McpCommand::List
                })
                .unwrap()
        );
        drop(runtime);
        let mut reopened = nola_application_bootstrap::open(&database).unwrap();
        assert_eq!(
            remaining,
            reopened
                .execute(ApplicationCommand::Mcp {
                    command: McpCommand::List
                })
                .unwrap()
        );
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn browser_baseline_requires_unique_positive_revisions() {
        assert!(mcp_changes_command(
            &[],
            &[json!({
                "id": "old-id", "display_name": "Old MCP", "endpoint": "https://example.test/mcp"
            })]
        )
        .is_err());
        assert!(mcp_changes_command(&[json!({"server_id":"first"})], &[]).is_err());
        assert!(mcp_changes_command(&[json!({"server_id":"first", "revision":0})], &[]).is_err());
        assert!(mcp_changes_command(
            &[
                json!({"server_id":"first", "revision":1}),
                json!({"server_id":"first", "revision":1}),
            ],
            &[]
        )
        .is_err());
    }
}
