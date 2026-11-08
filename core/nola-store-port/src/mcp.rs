// SPDX-License-Identifier: Apache-2.0
//! Mutations participate in the caller's unit of work, including sync publication.
use serde::{Deserialize, Serialize};

/// Host-owned MCP connection definition. `configuration_json` preserves the
/// transport-specific connection and the last successful capability snapshot;
/// Core receives only typed descriptors derived from this record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMcpServerRecord {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub configuration_json: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

pub trait McpRepository {
    type Error: std::fmt::Display;
    fn has_write_unit_of_work(&self) -> bool;
    fn mcp_server_identity_exists(&self, id: &str) -> Result<bool, Self::Error>;
    fn mcp_server(&self, id: &str) -> Result<Option<StoredMcpServerRecord>, Self::Error>;
    fn list_mcp_servers(&self) -> Result<Vec<StoredMcpServerRecord>, Self::Error>;
    fn upsert_mcp_server(
        &mut self,
        input: &StoredMcpServerRecord,
    ) -> Result<StoredMcpServerRecord, Self::Error>;
    fn delete_mcp_server(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error>;
}

/// Composition port that lends only MCP registry persistence for one use case.
pub trait McpRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: McpRepository<Error = Self::Error>
    where
        Self: 'a;

    fn mcp_repository(&mut self) -> Self::Repository<'_>;
}
