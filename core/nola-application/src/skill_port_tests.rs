// SPDX-License-Identifier: Apache-2.0
use crate::skill::{execute_skill_command, SkillCommand, SkillInput, SkillSource};
use nola_store_port::skill::*;
use std::collections::BTreeMap;

#[derive(Default, Clone)]
struct Repository {
    admitted: bool,
    rows: BTreeMap<String, (StoredSkill, i64)>,
    versions: BTreeMap<(String, String), UpsertSkillInput>,
    selections: BTreeMap<String, SkillInstructionSelection>,
    visibility: Option<SkillVisibilitySetting>,
    fail_selection: bool,
}
impl SkillRepository for Repository {
    type Error = String;
    fn has_write_unit_of_work(&self) -> bool {
        self.admitted
    }
    fn skill_editor_snapshot(&self, id: &str) -> Result<Option<(StoredSkill, i64)>, String> {
        Ok(self.rows.get(id).cloned())
    }
    fn list_skill_editors(&self) -> Result<Vec<(StoredSkill, i64)>, String> {
        Ok(self.rows.values().cloned().collect())
    }
    fn list_skills(&self, include_deleted: bool) -> Result<Vec<StoredSkill>, String> {
        Ok(self
            .rows
            .values()
            .map(|v| v.0.clone())
            .filter(|v| include_deleted || v.deleted_at_ms.is_none())
            .collect())
    }
    fn register_skill_version(&mut self, input: &UpsertSkillInput) -> Result<(), String> {
        let key = (input.id.clone(), input.version.clone());
        if self
            .versions
            .get(&key)
            .is_some_and(|old| old.instructions_text != input.instructions_text)
        {
            return Err("immutable version conflict".into());
        }
        self.versions.insert(key, input.clone());
        Ok(())
    }
    fn upsert_skill(&mut self, input: &UpsertSkillInput) -> Result<StoredSkill, String> {
        self.register_skill_version(input)?;
        let mut value = serde_json::to_value(input).unwrap();
        value["deleted_at_ms"] = serde_json::Value::Null;
        let stored: StoredSkill = serde_json::from_value(value).unwrap();
        let revision = self.rows.get(&input.id).map_or(1, |v| v.1 + 1);
        self.rows
            .insert(input.id.clone(), (stored.clone(), revision));
        Ok(stored)
    }
    fn set_skill_enabled(&mut self, id: &str, enabled: bool, _: i64) -> Result<bool, String> {
        let Some(v) = self.rows.get_mut(id) else {
            return Ok(false);
        };
        v.0.enabled = enabled;
        v.1 += 1;
        Ok(true)
    }
    fn delete_user_skill(&mut self, id: &str, at: i64) -> Result<bool, String> {
        let Some(v) = self.rows.get_mut(id) else {
            return Ok(false);
        };
        if v.0.source != SkillSourceKind::User {
            return Err("built-in".into());
        };
        v.0.deleted_at_ms = Some(at);
        v.1 += 1;
        Ok(true)
    }
    fn skill_instruction_selection(
        &self,
        id: &str,
    ) -> Result<Option<SkillInstructionSelection>, String> {
        Ok(self.selections.get(id).cloned())
    }
    fn set_skill_instruction_selection(
        &mut self,
        id: &str,
        value: &SkillInstructionSelection,
        _: i64,
    ) -> Result<(), String> {
        if self.fail_selection {
            return Err("selection sync failed".into());
        };
        self.selections.insert(id.into(), value.clone());
        Ok(())
    }
    fn skill_visibility(&self) -> Result<Option<SkillVisibilitySetting>, String> {
        Ok(self.visibility.clone())
    }
    fn put_skill_visibility(&mut self, json: &str, _: i64) -> Result<(), String> {
        self.visibility = Some(SkillVisibilitySetting {
            revision: self.visibility.as_ref().map_or(1, |v| v.revision + 1),
            value_json: json.into(),
        });
        Ok(())
    }
}
fn input(source: SkillSource) -> SkillInput {
    SkillInput {
        id: "skill".into(),
        source,
        version: "v1".into(),
        display_name: "Skill".into(),
        summary: "summary".into(),
        category: "test".into(),
        instructions_ref: "bundle://skill".into(),
        instructions_digest: "digest".into(),
        instructions_text: "  original\n🙂  ".into(),
        required_tools: vec![],
        default_mode: None,
        enabled: true,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[test]
fn non_sqlite_editor_catalog_revision_and_admission() {
    let mut repository = Repository::default();
    assert!(execute_skill_command(
        &mut repository,
        SkillCommand::CreateUser {
            input: input(SkillSource::User)
        }
    )
    .unwrap_err()
    .contains("admission"));
    repository.admitted = true;
    execute_skill_command(
        &mut repository,
        SkillCommand::CreateUser {
            input: input(SkillSource::User),
        },
    )
    .unwrap();
    assert!(execute_skill_command(
        &mut repository,
        SkillCommand::SetEnabledChecked {
            id: "skill".into(),
            enabled: false,
            expected_revision: 9,
            updated_at_ms: 2
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));
    let catalog = crate::load_skill_catalog(&repository).unwrap();
    assert_eq!(catalog.skills[0].instructions_text, "  original\n🙂  ");
    assert_eq!(catalog.descriptors.len(), 1);
    execute_skill_command(
        &mut repository,
        SkillCommand::SetEnabledChecked {
            id: "skill".into(),
            enabled: false,
            expected_revision: 1,
            updated_at_ms: 2,
        },
    )
    .unwrap();
    assert!(crate::load_skill_catalog(&repository)
        .unwrap()
        .descriptors
        .is_empty());
    execute_skill_command(
        &mut repository,
        SkillCommand::DeleteUserChecked {
            id: "skill".into(),
            expected_revision: 2,
            deleted_at_ms: 3,
        },
    )
    .unwrap();
    assert!(crate::load_skill_catalog(&repository)
        .unwrap()
        .skills
        .is_empty());
}

#[test]
fn non_sqlite_package_selection_preserves_override_and_propagates_sync_failure() {
    let mut repository = Repository {
        admitted: true,
        ..Default::default()
    };
    execute_skill_command(
        &mut repository,
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn)],
        },
    )
    .unwrap();
    execute_skill_command(
        &mut repository,
        SkillCommand::SetBuiltInInstructionsChecked {
            id: "skill".into(),
            expected_revision: 1,
            instructions_ref: "override".into(),
            instructions_digest: "override-v1".into(),
            instructions_text: " exact override\n".into(),
            updated_at_ms: 2,
        },
    )
    .unwrap();
    let mut package = input(SkillSource::BuiltIn);
    package.version = "v2".into();
    package.instructions_text = "new package".into();
    execute_skill_command(
        &mut repository,
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![package],
        },
    )
    .unwrap();
    assert_eq!(repository.selections["skill"].package_version, "v2");
    assert_eq!(
        crate::load_skill_catalog(&repository).unwrap().skills[0].instructions_text,
        " exact override\n"
    );
    repository.fail_selection = true;
    let failure = execute_skill_command(
        &mut repository,
        SkillCommand::SetBuiltInInstructionsChecked {
            id: "skill".into(),
            expected_revision: 2,
            instructions_ref: "override".into(),
            instructions_digest: "override-v2".into(),
            instructions_text: "next override".into(),
            updated_at_ms: 3,
        },
    )
    .unwrap_err();
    assert!(failure.contains("selection sync failed"));
}
