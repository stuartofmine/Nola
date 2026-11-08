// SPDX-License-Identifier: Apache-2.0
//! Narrow SQLite MCP repository sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::mcp::*;

/// Borrowed MCP registry capability; it cannot be retained as a second writer.
///
/// ```compile_fail
/// use nola_store::{McpRepository, NolaStore};
/// fn requires_mcp(_: &mut impl McpRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_mcp(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::skill::SkillRepository;
/// fn requires_skill(_: &mut impl SkillRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut servers = store.mcp_repository();
/// requires_skill(&mut servers);
/// ```
pub struct SqliteMcpRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn mcp_repository(&mut self) -> SqliteMcpRepository<'_> {
        SqliteMcpRepository { store: self }
    }
}

impl McpRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteMcpRepository<'a>;

    fn mcp_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::mcp_repository(self)
    }
}

impl McpRepository for SqliteMcpRepository<'_> {
    type Error = StoreError;
    fn has_write_unit_of_work(&self) -> bool {
        self.store.has_write_unit_of_work()
    }
    fn mcp_server_identity_exists(&self, id: &str) -> Result<bool, Self::Error> {
        self.store.mcp_server_identity_exists(id)
    }
    fn mcp_server(&self, id: &str) -> Result<Option<StoredMcpServerRecord>, Self::Error> {
        self.store.mcp_server(id)
    }
    fn list_mcp_servers(&self) -> Result<Vec<StoredMcpServerRecord>, Self::Error> {
        self.store.list_mcp_servers()
    }
    fn upsert_mcp_server(
        &mut self,
        input: &StoredMcpServerRecord,
    ) -> Result<StoredMcpServerRecord, Self::Error> {
        self.store.upsert_mcp_server(input)
    }
    fn delete_mcp_server(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        self.store.delete_mcp_server(id, deleted_at_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, updated_at_ms: i64) -> StoredMcpServerRecord {
        StoredMcpServerRecord {
            id: id.to_string(),
            display_name: "Test MCP".to_string(),
            enabled: true,
            configuration_json: r#"{"schemaVersion":1,"transport":"stdio"}"#.to_string(),
            revision: 1,
            created_at_ms: 10,
            updated_at_ms,
            deleted_at_ms: None,
        }
    }

    #[test]
    fn narrow_repository_reads_and_writes_inside_the_owner_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store
            .begin_write_unit_of_work()
            .expect("begin unit of work");
        {
            let mut servers = store.mcp_repository();
            servers
                .upsert_mcp_server(&server("mcp-a", 10))
                .expect("write MCP through narrow repository");
            assert!(servers
                .mcp_server_identity_exists("mcp-a")
                .expect("read MCP identity"));
        }
        store
            .rollback_write_unit_of_work()
            .expect("rollback unit of work");
        assert!(store
            .mcp_repository()
            .list_mcp_servers()
            .expect("read after rollback")
            .is_empty());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .mcp_repository()
            .upsert_mcp_server(&server("mcp-a", 20))
            .expect("write committed MCP");
        store.commit_write_unit_of_work().expect("commit");
        assert_eq!(
            store
                .mcp_repository()
                .list_mcp_servers()
                .expect("read committed MCP")
                .len(),
            1
        );
    }
}
