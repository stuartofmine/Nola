// SPDX-License-Identifier: Apache-2.0
//! Skill persistence contract. Mutations join the caller's admitted unit of work;
//! immutable instruction versions, selections and sync facts share its outcome.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    BuiltIn,
    User,
}

/// Registers one immutable Skill version and selects it as the current version.
/// Reusing `(skill_id, version)` with different content is a Store conflict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpsertSkillInput {
    pub id: String,
    pub source: SkillSourceKind,
    pub version: String,
    pub display_name: String,
    pub summary: String,
    pub category: String,
    pub instructions_ref: String,
    pub instructions_digest: String,
    pub instructions_text: String,
    #[serde(default)]
    pub required_tools_json: Option<String>,
    #[serde(default)]
    pub default_mode: Option<String>,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSkill {
    pub id: String,
    pub source: SkillSourceKind,
    pub version: String,
    pub display_name: String,
    pub summary: String,
    pub category: String,
    pub instructions_ref: String,
    pub instructions_digest: String,
    pub instructions_text: String,
    pub required_tools_json: Option<String>,
    pub default_mode: Option<String>,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInstructionSelection {
    pub package_version: String,
    pub override_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillVisibilitySetting {
    pub revision: i64,
    pub value_json: String,
}

pub trait SkillRepository {
    type Error: std::fmt::Display;
    fn has_write_unit_of_work(&self) -> bool;
    fn skill_editor_snapshot(&self, id: &str) -> Result<Option<(StoredSkill, i64)>, Self::Error>;
    fn list_skill_editors(&self) -> Result<Vec<(StoredSkill, i64)>, Self::Error>;
    fn list_skills(&self, include_deleted: bool) -> Result<Vec<StoredSkill>, Self::Error>;
    fn upsert_skill(&mut self, input: &UpsertSkillInput) -> Result<StoredSkill, Self::Error>;
    fn register_skill_version(&mut self, input: &UpsertSkillInput) -> Result<(), Self::Error>;
    fn set_skill_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn delete_user_skill(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error>;
    fn skill_instruction_selection(
        &self,
        id: &str,
    ) -> Result<Option<SkillInstructionSelection>, Self::Error>;
    fn set_skill_instruction_selection(
        &mut self,
        id: &str,
        selection: &SkillInstructionSelection,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error>;
    fn skill_visibility(&self) -> Result<Option<SkillVisibilitySetting>, Self::Error>;
    fn put_skill_visibility(
        &mut self,
        value_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error>;
}

/// Composition port that lends only Skill persistence for one use case.
pub trait SkillRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: SkillRepository<Error = Self::Error>
    where
        Self: 'a;

    fn skill_repository(&mut self) -> Self::Repository<'_>;
}
