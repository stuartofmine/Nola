// SPDX-License-Identifier: Apache-2.0
use super::*;
use std::collections::BTreeMap;
#[derive(Default)]
struct Repository {
    admitted: bool,
    rows: BTreeMap<String, StoredMcpServerRecord>,
    fail_sync: bool,
}
impl McpRepository for Repository {
    type Error = String;
    fn has_write_unit_of_work(&self) -> bool {
        self.admitted
    }
    fn mcp_server_identity_exists(&self, id: &str) -> Result<bool, String> {
        Ok(self.rows.contains_key(id))
    }
    fn mcp_server(&self, id: &str) -> Result<Option<StoredMcpServerRecord>, String> {
        Ok(self
            .rows
            .get(id)
            .filter(|v| v.deleted_at_ms.is_none())
            .cloned())
    }
    fn list_mcp_servers(&self) -> Result<Vec<StoredMcpServerRecord>, String> {
        Ok(self
            .rows
            .values()
            .filter(|v| v.deleted_at_ms.is_none())
            .cloned()
            .collect())
    }
    fn upsert_mcp_server(
        &mut self,
        input: &StoredMcpServerRecord,
    ) -> Result<StoredMcpServerRecord, String> {
        if self.fail_sync {
            return Err("MCP sync failed".into());
        };
        let mut row = input.clone();
        row.revision += 1;
        self.rows.insert(row.id.clone(), row.clone());
        Ok(row)
    }
    fn delete_mcp_server(&mut self, id: &str, at: i64) -> Result<bool, String> {
        let Some(row) = self.rows.get_mut(id) else {
            return Ok(false);
        };
        row.deleted_at_ms = Some(at);
        row.revision += 1;
        Ok(true)
    }
}
fn create() -> McpCommand {
    McpCommand::Create {
        id: "server".into(),
        display_name: "Server".into(),
        enabled: true,
        configuration_json: r#"{"transport":"stdio","command":"nola"}"#.into(),
        created_at_ms: 1,
    }
}
#[test]
fn non_sqlite_mcp_admission_revision_failure_and_tombstone_identity() {
    let mut repository = Repository::default();
    assert!(execute_mcp_command(&mut repository, create())
        .unwrap_err()
        .contains("admission"));
    repository.admitted = true;
    let original = execute_mcp_command(&mut repository, create()).unwrap();
    assert!(execute_mcp_command(&mut repository, create())
        .unwrap_err()
        .contains("identity_conflict"));
    let edit = |revision| McpCommand::Configure {
        id: "server".into(),
        expected_revision: revision,
        display_name: "Changed".into(),
        enabled: false,
        configuration_json: "{}".into(),
        updated_at_ms: 2,
    };
    assert!(execute_mcp_command(&mut repository, edit(9))
        .unwrap_err()
        .contains("revision_conflict"));
    repository.fail_sync = true;
    assert!(execute_mcp_command(&mut repository, edit(1))
        .unwrap_err()
        .contains("sync failed"));
    assert_eq!(
        execute_mcp_command(&mut repository, McpCommand::List).unwrap(),
        json!([original])
    );
    repository.fail_sync = false;
    let updated = execute_mcp_command(&mut repository, edit(1)).unwrap();
    assert_eq!(updated["revision"], 2);
    execute_mcp_command(
        &mut repository,
        McpCommand::DeleteChecked {
            id: "server".into(),
            expected_revision: 2,
            deleted_at_ms: 3,
        },
    )
    .unwrap();
    assert_eq!(
        execute_mcp_command(&mut repository, McpCommand::List).unwrap(),
        json!([])
    );
    assert!(execute_mcp_command(&mut repository, create())
        .unwrap_err()
        .contains("identity_conflict"));
}
