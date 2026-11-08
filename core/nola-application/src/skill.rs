// SPDX-License-Identifier: Apache-2.0
//! Skill product use-case boundary.
//!
//! Application exposes this capability as one cohesive port instead of a set
//! of unrelated Store-shaped top-level commands.

use nola_agent_engine::agent::capability::{CapabilityToolReference, SkillExecutionMode};
use nola_store_port::skill::{SkillRepository, SkillSourceKind, StoredSkill, UpsertSkillInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    BuiltIn,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillInput {
    pub id: String,
    pub source: SkillSource,
    pub version: String,
    pub display_name: String,
    pub summary: String,
    pub category: String,
    pub instructions_ref: String,
    pub instructions_digest: String,
    pub instructions_text: String,
    #[serde(default)]
    pub required_tools: Vec<CapabilityToolReference>,
    pub default_mode: Option<SkillExecutionMode>,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    pub source: SkillSource,
    pub version: String,
    pub display_name: String,
    pub summary: String,
    pub category: String,
    pub instructions_ref: String,
    pub instructions_digest: String,
    pub instructions_text: String,
    #[serde(default)]
    pub required_tools: Vec<CapabilityToolReference>,
    pub default_mode: Option<SkillExecutionMode>,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCommand {
    ConfigureBuiltIn {
        id: String,
        expected_revision: i64,
        instructions_text: String,
        enabled: bool,
        updated_at_ms: i64,
    },
    SetBuiltInInstructionsChecked {
        id: String,
        expected_revision: i64,
        instructions_ref: String,
        instructions_digest: String,
        instructions_text: String,
        updated_at_ms: i64,
    },
    SynchronizeBuiltIns {
        inputs: Vec<SkillInput>,
    },
    ListEditors,
    RemoveBuiltIn {
        id: String,
        expected_revision: i64,
        expected_visibility_revision: Option<i64>,
        updated_at_ms: i64,
    },
    RestoreBuiltIns {
        expected_revisions: std::collections::BTreeMap<String, i64>,
        expected_visibility_revision: Option<i64>,
        updated_at_ms: i64,
    },
    ApplyChanges {
        commands: Vec<SkillCommand>,
    },
    GetEditor {
        id: String,
    },
    CreateUser {
        input: SkillInput,
    },
    ConfigureUser {
        input: SkillInput,
        expected_revision: i64,
    },
    SetEnabledChecked {
        id: String,
        enabled: bool,
        expected_revision: i64,
        updated_at_ms: i64,
    },
    DeleteUserChecked {
        id: String,
        expected_revision: i64,
        deleted_at_ms: i64,
    },
    Upsert {
        input: SkillInput,
    },
    List,
    LoadCatalog,
    SetEnabled {
        id: String,
        enabled: bool,
        updated_at_ms: i64,
    },
    DeleteUser {
        id: String,
        deleted_at_ms: i64,
    },
}

pub(crate) fn execute_skill_command(
    store: &mut impl SkillRepository,
    command: SkillCommand,
) -> Result<Value, String> {
    match command {
        SkillCommand::SynchronizeBuiltIns { inputs } => {
            require_editor_admission(store)?;
            if inputs.len() > 128 {
                return Err("built-in Skill synchronization is limited to 128 entries".into());
            }
            let mut ids = std::collections::BTreeSet::new();
            for input in &inputs {
                if input.source != SkillSource::BuiltIn || !ids.insert(input.id.clone()) {
                    return Err(
                        "built-in synchronization requires unique built-in Skill identities".into(),
                    );
                }
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Visibility {
                schema_version: u64,
                ids: std::collections::BTreeSet<String>,
            }
            let hidden = match store
                .skill_visibility()
                .map_err(|error| error.to_string())?
            {
                Some(setting) => {
                    let visibility: Visibility = serde_json::from_str(&setting.value_json)
                        .map_err(|error| format!("invalid built-in Skill visibility: {error}"))?;
                    if visibility.schema_version != 1 {
                        return Err("unsupported built-in Skill visibility schema".into());
                    }
                    visibility.ids
                }
                None => Default::default(),
            };
            let mut editors = Vec::with_capacity(inputs.len());
            for mut input in inputs {
                let package_version = input.version.clone();
                let selection = store
                    .skill_instruction_selection(&input.id)
                    .map_err(|error| error.to_string())?;
                let updated_selection = nola_store_port::skill::SkillInstructionSelection {
                    package_version,
                    override_version: selection.and_then(|value| value.override_version),
                };
                if let Some((record, revision)) = store
                    .skill_editor_snapshot(&input.id)
                    .map_err(|error| error.to_string())?
                {
                    let current = Skill::try_from(record)?;
                    if current.source != SkillSource::BuiltIn || current.deleted_at_ms.is_some() {
                        return Err(format!("skill_identity_conflict: {}", input.id));
                    }
                    // Package metadata cannot overwrite user-owned activation.
                    input.enabled = current.enabled;
                    input.created_at_ms = current.created_at_ms;
                    input.updated_at_ms = input.updated_at_ms.max(current.updated_at_ms);
                    if updated_selection.override_version.is_some() {
                        store
                            .register_skill_version(&UpsertSkillInput::try_from(input.clone())?)
                            .map_err(|error| error.to_string())?;
                        input.version = current.version.clone();
                        input.instructions_ref = current.instructions_ref.clone();
                        input.instructions_digest = current.instructions_digest.clone();
                        input.instructions_text = current.instructions_text.clone();
                        input.required_tools = current.required_tools.clone();
                        input.default_mode = current.default_mode;
                    }
                    // A new startup request is not a package change. Preserve
                    // editor revisions and timestamps when package-owned facts
                    // are identical, even if filesystem mtimes have changed.
                    if input.version == current.version
                        && input.display_name == current.display_name
                        && input.summary == current.summary
                        && input.category == current.category
                        && input.instructions_ref == current.instructions_ref
                        && input.instructions_digest == current.instructions_digest
                        && input.instructions_text == current.instructions_text
                        && input.required_tools == current.required_tools
                        && input.default_mode == current.default_mode
                    {
                        let _ = revision;
                        store
                            .set_skill_instruction_selection(
                                &input.id,
                                &updated_selection,
                                input.updated_at_ms,
                            )
                            .map_err(|error| error.to_string())?;
                        editors.push(execute_skill_command(
                            store,
                            SkillCommand::GetEditor { id: input.id },
                        )?);
                        continue;
                    }
                } else {
                    input.enabled = input.enabled && !hidden.contains(&input.id);
                }
                let id = input.id.clone();
                let updated_at_ms = input.updated_at_ms;
                crate::register_skill(store, input)?;
                store
                    .set_skill_instruction_selection(&id, &updated_selection, updated_at_ms)
                    .map_err(|error| error.to_string())?;
                editors.push(execute_skill_command(
                    store,
                    SkillCommand::GetEditor { id },
                )?);
            }
            Ok(Value::Array(editors))
        }
        SkillCommand::ListEditors => store
            .list_skill_editors()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|(stored, revision)| {
                Skill::try_from(stored)
                    .map(|record| json!({"record": record, "revision": revision}))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        SkillCommand::RemoveBuiltIn {
            id,
            expected_revision,
            expected_visibility_revision,
            updated_at_ms,
        } => {
            let current = editable_skill(store, &id, expected_revision)?;
            if current.source != SkillSource::BuiltIn {
                return Err("only built-in Skills can be removed from the built-in library".into());
            }
            let setting = store
                .skill_visibility()
                .map_err(|error| error.to_string())?;
            if setting.as_ref().map(|value| value.revision) != expected_visibility_revision {
                return Err("skill_visibility_revision_conflict".into());
            }
            #[derive(Serialize, Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Visibility {
                schema_version: u64,
                ids: std::collections::BTreeSet<String>,
            }
            let mut visibility = match setting {
                Some(setting) => serde_json::from_str::<Visibility>(&setting.value_json)
                    .map_err(|error| format!("invalid built-in Skill visibility: {error}"))?,
                None => Visibility {
                    schema_version: 1,
                    ids: Default::default(),
                },
            };
            if visibility.schema_version != 1 {
                return Err("unsupported built-in Skill visibility schema".into());
            }
            visibility.ids.insert(id.clone());
            let value = serde_json::to_string(&visibility).map_err(|error| error.to_string())?;
            crate::set_skill_enabled(store, &id, false, updated_at_ms)?;
            store
                .put_skill_visibility(&value, updated_at_ms)
                .map_err(|error| error.to_string())?;
            execute_skill_command(store, SkillCommand::GetEditor { id })
        }
        SkillCommand::RestoreBuiltIns {
            expected_revisions,
            expected_visibility_revision,
            updated_at_ms,
        } => {
            require_editor_admission(store)?;
            let setting = store
                .skill_visibility()
                .map_err(|error| error.to_string())?;
            if setting.as_ref().map(|value| value.revision) != expected_visibility_revision {
                return Err("skill_visibility_revision_conflict".into());
            }
            let ids = store
                .list_skills(false)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|record| matches!(record.source, SkillSourceKind::BuiltIn))
                .map(|record| record.id)
                .collect::<std::collections::BTreeSet<_>>();
            if ids != expected_revisions.keys().cloned().collect() {
                return Err("skill_catalog_conflict: reload the complete built-in catalog".into());
            }
            let mut restored = Vec::with_capacity(ids.len());
            for (id, expected_revision) in expected_revisions {
                restored.push(execute_skill_command(
                    store,
                    SkillCommand::SetEnabledChecked {
                        id,
                        enabled: true,
                        expected_revision,
                        updated_at_ms,
                    },
                )?);
            }
            store
                .put_skill_visibility(r#"{"schemaVersion":1,"ids":[]}"#, updated_at_ms)
                .map_err(|error| error.to_string())?;
            Ok(Value::Array(restored))
        }
        SkillCommand::ApplyChanges { commands } => {
            require_editor_admission(store)?;
            if commands.len() > 128 {
                return Err("Skill changes are limited to 128 operations".into());
            }
            if commands.iter().any(|command| {
                !matches!(
                    command,
                    SkillCommand::ConfigureBuiltIn { .. }
                        | SkillCommand::CreateUser { .. }
                        | SkillCommand::ConfigureUser { .. }
                        | SkillCommand::SetEnabledChecked { .. }
                        | SkillCommand::DeleteUserChecked { .. }
                )
            }) {
                return Err("Skill changes only accept checked built-in and user editor commands".into());
            }
            commands
                .into_iter()
                .map(|command| execute_skill_command(store, command))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        SkillCommand::GetEditor { id } => {
            let snapshot = store
                .skill_editor_snapshot(&id)
                .map_err(|error| error.to_string())?;
            snapshot
                .map(|(record, revision)| {
                    Skill::try_from(record)
                        .map(|record| json!({"record": record, "revision": revision}))
                })
                .transpose()
                .map(|value| value.unwrap_or(Value::Null))
        }
        SkillCommand::CreateUser { input } => {
            require_editor_admission(store)?;
            if input.source != SkillSource::User {
                return Err("user Skill creation cannot register built-in content".into());
            }
            if store
                .skill_editor_snapshot(&input.id)
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(format!("skill_identity_conflict: {}", input.id));
            }
            let id = input.id.clone();
            crate::register_skill(store, input)?;
            execute_skill_command(store, SkillCommand::GetEditor { id })
        }
        SkillCommand::ConfigureBuiltIn { id, expected_revision, instructions_text, enabled, updated_at_ms } => {
            let current = editable_skill(store, &id, expected_revision)?;
            if current.source != SkillSource::BuiltIn {
                return Err("built-in editing requires a built-in Skill".into());
            }
            let mut editor = if current.instructions_text != instructions_text {
                use sha2::{Digest, Sha256};
                let digest = format!("{:x}", Sha256::digest(instructions_text.as_bytes()));
                execute_skill_command(store, SkillCommand::SetBuiltInInstructionsChecked {
                    id: id.clone(), expected_revision,
                    instructions_ref: format!("nola://skills/{id}/{digest}"),
                    instructions_digest: digest, instructions_text, updated_at_ms,
                })?
            } else {
                execute_skill_command(store, SkillCommand::GetEditor { id: id.clone() })?
            };
            if current.enabled != enabled {
                editor = execute_skill_command(store, SkillCommand::SetEnabledChecked {
                    id, enabled, expected_revision: editor["revision"].as_i64().ok_or("Skill editor revision missing")?, updated_at_ms,
                })?;
            }
            Ok(editor)
        }
        SkillCommand::SetBuiltInInstructionsChecked {
            id,
            expected_revision,
            instructions_ref,
            instructions_digest,
            instructions_text,
            updated_at_ms,
        } => {
            let current = editable_skill(store, &id, expected_revision)?;
            if current.source != SkillSource::BuiltIn {
                return Err("built-in instruction editing requires a built-in Skill".into());
            }
            if instructions_text.trim().is_empty() || instructions_text.len() > 2 * 1024 * 1024 {
                return Err("built-in instructions must be nonempty and at most 2 MiB".into());
            }
            let package_version = store
                .skill_instruction_selection(&id)
                .map_err(|error| error.to_string())?
                .map(|selection| selection.package_version)
                .unwrap_or_else(|| current.version.clone());
            let selection = nola_store_port::skill::SkillInstructionSelection {
                package_version,
                override_version: Some(instructions_digest.clone()),
            };
            crate::register_skill(
                store,
                SkillInput {
                    id: id.clone(),
                    source: current.source,
                    version: instructions_digest.clone(),
                    display_name: current.display_name,
                    summary: current.summary,
                    category: current.category,
                    instructions_ref,
                    instructions_digest,
                    instructions_text,
                    required_tools: current.required_tools,
                    default_mode: current.default_mode,
                    enabled: current.enabled,
                    created_at_ms: current.created_at_ms,
                    updated_at_ms: updated_at_ms.max(current.updated_at_ms),
                },
            )?;
            store
                .set_skill_instruction_selection(&id, &selection, updated_at_ms)
                .map_err(|error| error.to_string())?;
            execute_skill_command(store, SkillCommand::GetEditor { id })
        }
        SkillCommand::ConfigureUser {
            mut input,
            expected_revision,
        } => {
            let current = editable_skill(store, &input.id, expected_revision)?;
            if current.source != SkillSource::User || input.source != SkillSource::User {
                return Err("built-in Skills cannot be edited through the user editor".into());
            }
            input.created_at_ms = current.created_at_ms;
            let id = input.id.clone();
            let enabled = input.enabled;
            let updated_at_ms = input.updated_at_ms;
            crate::register_skill(store, input)?;
            crate::set_skill_enabled(store, &id, enabled, updated_at_ms)?;
            execute_skill_command(store, SkillCommand::GetEditor { id })
        }
        SkillCommand::SetEnabledChecked {
            id,
            enabled,
            expected_revision,
            updated_at_ms,
        } => {
            editable_skill(store, &id, expected_revision)?;
            crate::set_skill_enabled(store, &id, enabled, updated_at_ms)?;
            execute_skill_command(store, SkillCommand::GetEditor { id })
        }
        SkillCommand::DeleteUserChecked {
            id,
            expected_revision,
            deleted_at_ms,
        } => {
            editable_skill(store, &id, expected_revision)?;
            Ok(json!(crate::delete_user_skill(store, &id, deleted_at_ms)?))
        }
        SkillCommand::Upsert { input } => {
            serde_json::to_value(crate::register_skill(store, input)?)
                .map_err(|error| format!("encode Skill result failed: {error}"))
        }
        SkillCommand::List => {
            let skills: Vec<Skill> = store
                .list_skills(false)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(Skill::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(skills)
                .map_err(|error| format!("encode Skill list failed: {error}"))
        }
        SkillCommand::LoadCatalog => serde_json::to_value(crate::load_skill_catalog(store)?)
            .map_err(|error| format!("encode Skill catalog failed: {error}")),
        SkillCommand::SetEnabled {
            id,
            enabled,
            updated_at_ms,
        } => Ok(json!(crate::set_skill_enabled(
            store,
            &id,
            enabled,
            updated_at_ms
        )?)),
        SkillCommand::DeleteUser { id, deleted_at_ms } => {
            Ok(json!(crate::delete_user_skill(store, &id, deleted_at_ms)?))
        }
    }
}

fn require_editor_admission(store: &impl SkillRepository) -> Result<(), String> {
    if !store.has_write_unit_of_work() {
        return Err("Skill editing requires durable command admission".into());
    }
    Ok(())
}

fn editable_skill(
    store: &impl SkillRepository,
    id: &str,
    expected_revision: i64,
) -> Result<Skill, String> {
    require_editor_admission(store)?;
    if expected_revision < 1 {
        return Err("Skill editing requires a positive expected revision".into());
    }
    let (stored, revision) = store
        .skill_editor_snapshot(id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Skill is missing: {id}"))?;
    let record = Skill::try_from(stored)?;
    if record.deleted_at_ms.is_some() {
        return Err(format!("Skill is deleted: {id}"));
    }
    if revision != expected_revision {
        return Err(format!(
            "skill_revision_conflict: {id}: expected {expected_revision}, actual {revision}"
        ));
    }
    Ok(record)
}

impl From<SkillSource> for SkillSourceKind {
    fn from(value: SkillSource) -> Self {
        match value {
            SkillSource::BuiltIn => Self::BuiltIn,
            SkillSource::User => Self::User,
        }
    }
}

impl From<SkillSourceKind> for SkillSource {
    fn from(value: SkillSourceKind) -> Self {
        match value {
            SkillSourceKind::BuiltIn => Self::BuiltIn,
            SkillSourceKind::User => Self::User,
        }
    }
}

impl TryFrom<SkillInput> for UpsertSkillInput {
    type Error = String;

    fn try_from(value: SkillInput) -> Result<Self, Self::Error> {
        let required_tools_json = if value.required_tools.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&value.required_tools)
                    .map_err(|error| format!("encode required Skill tools failed: {error}"))?,
            )
        };
        Ok(Self {
            id: value.id,
            source: value.source.into(),
            version: value.version,
            display_name: value.display_name,
            summary: value.summary,
            category: value.category,
            instructions_ref: value.instructions_ref,
            instructions_digest: value.instructions_digest,
            instructions_text: value.instructions_text,
            required_tools_json,
            default_mode: value.default_mode.map(|mode| match mode {
                SkillExecutionMode::Inline => "inline".to_string(),
                SkillExecutionMode::Fork => "fork".to_string(),
            }),
            enabled: value.enabled,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
        })
    }
}

impl TryFrom<StoredSkill> for Skill {
    type Error = String;

    fn try_from(value: StoredSkill) -> Result<Self, Self::Error> {
        let required_tools = value
            .required_tools_json
            .as_deref()
            .map(serde_json::from_str::<Vec<CapabilityToolReference>>)
            .transpose()
            .map_err(|error| format!("decode persisted Skill tools failed: {error}"))?
            .unwrap_or_default();
        let default_mode = match value.default_mode.as_deref() {
            Some("inline") => Some(SkillExecutionMode::Inline),
            Some("fork") => Some(SkillExecutionMode::Fork),
            None => None,
            Some(mode) => return Err(format!("unsupported persisted Skill mode: {mode}")),
        };
        Ok(Self {
            id: value.id,
            source: value.source.into(),
            version: value.version,
            display_name: value.display_name,
            summary: value.summary,
            category: value.category,
            instructions_ref: value.instructions_ref,
            instructions_digest: value.instructions_digest,
            instructions_text: value.instructions_text,
            required_tools,
            default_mode,
            enabled: value.enabled,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        })
    }
}

#[cfg(test)]
#[path = "skill_port_tests.rs"]
mod port_tests;
