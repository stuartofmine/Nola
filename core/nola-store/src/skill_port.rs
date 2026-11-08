// SPDX-License-Identifier: Apache-2.0
//! Narrow SQLite Skill repository sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::skill::*;

/// Borrowed Skill capability; it cannot be retained as a second writer.
///
/// ```compile_fail
/// use nola_store::{NolaStore, SkillRepository};
/// fn requires_skill(_: &mut impl SkillRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_skill(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::model::ModelRepository;
/// fn requires_model(_: &mut impl ModelRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut skills = store.skill_repository();
/// requires_model(&mut skills);
/// ```
pub struct SqliteSkillRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn skill_repository(&mut self) -> SqliteSkillRepository<'_> {
        SqliteSkillRepository { store: self }
    }
}

impl SkillRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteSkillRepository<'a>;

    fn skill_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::skill_repository(self)
    }
}

impl SkillRepository for SqliteSkillRepository<'_> {
    type Error = StoreError;
    fn has_write_unit_of_work(&self) -> bool {
        self.store.has_write_unit_of_work()
    }
    fn skill_editor_snapshot(&self, id: &str) -> Result<Option<(StoredSkill, i64)>, Self::Error> {
        self.store.skill_editor_snapshot(id)
    }
    fn list_skill_editors(&self) -> Result<Vec<(StoredSkill, i64)>, Self::Error> {
        self.store.list_skill_editors()
    }
    fn list_skills(&self, include_deleted: bool) -> Result<Vec<StoredSkill>, Self::Error> {
        self.store.list_skills(include_deleted)
    }
    fn upsert_skill(&mut self, input: &UpsertSkillInput) -> Result<StoredSkill, Self::Error> {
        self.store.upsert_skill(input)
    }
    fn register_skill_version(&mut self, input: &UpsertSkillInput) -> Result<(), Self::Error> {
        self.store.register_skill_version(input)
    }
    fn set_skill_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store.set_skill_enabled(id, enabled, updated_at_ms)
    }
    fn delete_user_skill(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        self.store.delete_user_skill(id, deleted_at_ms)
    }
    fn skill_instruction_selection(
        &self,
        id: &str,
    ) -> Result<Option<SkillInstructionSelection>, Self::Error> {
        self.store.skill_instruction_selection(id)
    }
    fn set_skill_instruction_selection(
        &mut self,
        id: &str,
        selection: &SkillInstructionSelection,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.store
            .set_skill_instruction_selection(id, selection, updated_at_ms)
    }
    fn skill_visibility(&self) -> Result<Option<SkillVisibilitySetting>, Self::Error> {
        self.store
            .product_setting("skill.builtin.hidden")
            .map(|value| {
                value.map(|setting| SkillVisibilitySetting {
                    revision: setting.revision,
                    value_json: setting.value_json,
                })
            })
    }
    fn put_skill_visibility(
        &mut self,
        value_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.store
            .put_product_setting("skill.builtin.hidden", value_json, updated_at_ms)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn skill(id: &str, updated_at_ms: i64) -> UpsertSkillInput {
        let instructions = "Use the typed test capability.";
        UpsertSkillInput {
            id: id.to_string(),
            source: SkillSourceKind::User,
            version: "1".to_string(),
            display_name: "Test Skill".to_string(),
            summary: "Contract fixture".to_string(),
            category: "test".to_string(),
            instructions_ref: "skill://test/instructions".to_string(),
            instructions_digest: format!("{:x}", Sha256::digest(instructions.as_bytes())),
            instructions_text: instructions.to_string(),
            required_tools_json: Some("[]".to_string()),
            default_mode: Some("inline".to_string()),
            enabled: true,
            created_at_ms: 10,
            updated_at_ms,
        }
    }

    #[test]
    fn narrow_repository_reads_and_writes_inside_the_owner_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store
            .begin_write_unit_of_work()
            .expect("begin unit of work");
        {
            let mut skills = store.skill_repository();
            skills
                .upsert_skill(&skill("skill-a", 10))
                .expect("write Skill through narrow repository");
            assert_eq!(skills.list_skills(false).expect("read Skill").len(), 1);
        }
        store
            .rollback_write_unit_of_work()
            .expect("rollback unit of work");
        assert!(store
            .skill_repository()
            .list_skills(false)
            .expect("read after rollback")
            .is_empty());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .skill_repository()
            .upsert_skill(&skill("skill-a", 20))
            .expect("write committed Skill");
        store.commit_write_unit_of_work().expect("commit");
        assert_eq!(
            store
                .skill_repository()
                .list_skills(false)
                .expect("read committed Skill")
                .len(),
            1
        );
    }
}
