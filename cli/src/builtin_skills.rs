use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) struct EmbeddedSkillFile {
    relative_path: &'static str,
    contents: &'static [u8],
}

pub(crate) struct EmbeddedSkill {
    id: &'static str,
    resource_dir: &'static str,
    title: &'static str,
    description: &'static str,
    instruction: &'static str,
    user_facing: bool,
    files: &'static [EmbeddedSkillFile],
}

const EMBEDDED_SKILLS: &[EmbeddedSkill] =
    include!(concat!(env!("OUT_DIR"), "/builtin_skills_generated.rs"));

pub(crate) fn user_facing_ids() -> Vec<&'static str> {
    EMBEDDED_SKILLS
        .iter()
        .filter(|skill| skill.user_facing)
        .map(|skill| skill.id)
        .collect()
}

pub(crate) fn invocation_id_schema() -> Value {
    json!({
        "type": "string",
        "description": "Exact canonical Skill id from the left side of an entry in the adjacent available-skills system reminder (NOLA_SKILL_CATALOG_V1). Never pass a display name or title.",
        "examples": ["analyze-data"]
    })
}

pub(crate) fn is_user_facing_id(id: &str) -> bool {
    if EMBEDDED_SKILLS
        .iter()
        .any(|skill| skill.user_facing && skill.id == id)
    {
        return true;
    }
    managed_directory_for_id(id)
        .ok()
        .flatten()
        .is_some_and(|directory| directory.join("SKILL.md").is_file())
}

pub(crate) fn managed_directory_for_id(id: &str) -> Result<Option<PathBuf>, String> {
    if let Some(skill) = EMBEDDED_SKILLS
        .iter()
        .find(|skill| skill.user_facing && skill.id == id)
    {
        return Ok(Some(managed_root()?.join(skill.resource_dir)));
    }
    if !valid_custom_skill_id(id) {
        return Ok(None);
    }
    let directory = managed_custom_root()?.join(id);
    Ok(directory.join("SKILL.md").is_file().then_some(directory))
}

pub(crate) fn materialize_managed_directory_for_id(id: &str) -> Result<Option<PathBuf>, String> {
    let Some(skill) = EMBEDDED_SKILLS
        .iter()
        .find(|skill| skill.user_facing && skill.id == id)
    else {
        return Ok(None);
    };
    let root = managed_root()?;
    // A standalone CLI has no settings screen that performs the initial
    // synchronization done by the Web and native clients. Bootstrap the
    // embedded defaults only when the managed root has never existed. Once
    // initialized, a missing directory remains authoritative disabled state,
    // so invoking a Skill cannot silently re-enable or overwrite it.
    if !root.exists() {
        materialize_enabled_at(&root, &[])?;
    }
    let directory = root.join(skill.resource_dir);
    // Activation consumes the version already materialized by settings.
    // Missing resources mean that the Skill is disabled or unavailable;
    // reinstalling the embedded default here would both erase edits and
    // silently re-enable a Skill the user explicitly disabled.
    Ok(directory.join("SKILL.md").is_file().then_some(directory))
}

pub(crate) fn managed_root() -> Result<PathBuf, String> {
    let home = crate::platform_paths::home_dir()
        .ok_or_else(|| "cannot resolve the user home directory".to_string())?;
    Ok(home
        .join(".nola")
        .join("cli")
        .join("skills")
        .join("built-in"))
}

pub(crate) fn managed_custom_root() -> Result<PathBuf, String> {
    let home = crate::platform_paths::home_dir()
        .ok_or_else(|| "cannot resolve the user home directory".to_string())?;
    Ok(home.join(".nola").join("cli").join("skills").join("custom"))
}

pub(crate) fn catalog(overrides: &[Value]) -> Result<Vec<Value>, String> {
    let root = managed_root()?;
    Ok(catalog_at(&root, overrides))
}

pub(crate) fn merged_catalog(
    overrides: &[Value],
    uploaded: &[Value],
) -> Result<Vec<Value>, String> {
    // These are this local user's embedded Skill selections/overrides, not
    // foreign resources. Restore their owner envelope before Core ACL projection.
    let builtin = catalog(overrides)?.iter().map(|item|
        crate::resource_access::restore_local_user_resource_identity(item,
            nola_core::agent::resource_access::AgentResourceKind::Skill)
    ).collect::<Result<Vec<_>, _>>()?;
    let mut items = builtin.into_iter()
        .filter_map(|item| {
            crate::resource_access::api_projection(
                &item,
                nola_core::agent::resource_access::AgentResourceKind::Skill,
            )
        })
        .collect::<Vec<_>>();
    items.extend(
        uploaded
            .iter()
            .cloned()
            .map(normalize_uploaded_skill)
            .filter_map(|item| {
                crate::resource_access::api_projection(
                    &item,
                    nola_core::agent::resource_access::AgentResourceKind::Skill,
                )
            }),
    );
    Ok(items)
}

pub(crate) fn context_index(overrides: &[Value], uploaded: &[Value]) -> Result<Vec<Value>, String> {
    let root = managed_root()?;
    let mut items = catalog_at(&root, overrides)
        .into_iter()
        .filter(|item| {
            item.get("enabled").and_then(Value::as_bool) != Some(false)
                && item.get("removed").and_then(Value::as_bool) != Some(true)
        })
        .map(|item| {
            json!({
                "id": item.get("id").cloned().unwrap_or(Value::Null),
                "name": item.get("name").cloned().unwrap_or(Value::Null),
                "description": item.get("description").cloned().unwrap_or(Value::Null),
                "enabled": true,
                "source": "built_in",
            })
        })
        .collect::<Vec<_>>();
    items.extend(
        uploaded
            .iter()
            .cloned()
            .map(normalize_uploaded_skill)
            .filter(|item| {
                crate::resource_access::allows(
                    item,
                    nola_core::agent::resource_access::AgentResourceKind::Skill,
                    nola_core::agent::resource_access::AgentResourcePermission::ReadMetadata,
                )
            })
            .filter(|item| {
                item.get("enabled").and_then(Value::as_bool) != Some(false)
                    && item.get("removed").and_then(Value::as_bool) != Some(true)
                    && item
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| !content.trim().is_empty())
            })
            .map(|item| {
                json!({
                    "id": item.get("id").cloned().unwrap_or(Value::Null),
                    "name": item.get("name").cloned().unwrap_or(Value::Null),
                    "description": item.get("description").cloned().unwrap_or(Value::Null),
                    "enabled": true,
                    "source": "custom",
                })
            }),
    );
    Ok(items)
}

pub(crate) fn catalog_prompt(overrides: &[Value], uploaded: &[Value]) -> Result<String, String> {
    catalog_prompt_from_index(&context_index(overrides, uploaded)?)
}

pub(crate) fn catalog_prompt_from_index(items: &[Value]) -> Result<String, String> {
    const DESCRIPTION_LIMIT: usize = 250;
    const CATALOG_LIMIT: usize = 8_000;
    let mut lines = vec![
        "NOLA_SKILL_CATALOG_V1".to_string(),
        "Each entry is `- <canonical skill_id>: <description>`. Pass the exact canonical skill_id to Skill.skill; display names and titles are not valid ids.".to_string(),
    ];
    let mut used = lines.iter().map(String::len).sum::<usize>();
    for item in items {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(DESCRIPTION_LIMIT)
            .collect::<String>();
        let line = format!("- {id}: {description}");
        if used + line.len() > CATALOG_LIMIT {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod discovery_tests {
    use super::{catalog_prompt, invocation_id_schema, managed_directory_for_id, EMBEDDED_SKILLS};
    use nola_core::agent::capability::{
        SkillDescriptor, SkillExecutionContract, SkillExecutionMode, SkillRunBudget, SkillRunOwner,
        SkillRuntimeCommand, SkillRuntimeCommandEnvelope, SkillRuntimeState,
    };
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    #[test]
    fn discovery_catalog_is_bounded_and_contains_no_full_skill_body() {
        let prompt = catalog_prompt(&[], &[]).expect("built-in Skill catalog");
        assert!(prompt.len() <= 8_000);
        assert!(prompt.starts_with("NOLA_SKILL_CATALOG_V1\n"));
        assert!(prompt.contains("Pass the exact canonical skill_id to Skill.skill"));
        assert!(prompt.contains("- extract-image-text:"));
        assert!(prompt.contains("- plan-schedule-tasks:"));
        assert!(!prompt.contains("call `skill_invoke`"));
        assert!(!prompt.contains("skill_skip"));
        assert!(!prompt.contains("extract_text.swift"));
        assert!(!prompt.contains("--validate-only"));
        assert!(!prompt.contains("recommended_id_for_prompt"));
    }

    #[test]
    fn stable_skill_id_resolves_its_actual_resource_directory() {
        let directory = managed_directory_for_id("excel-workbook-authoring")
            .expect("managed root")
            .expect("known Skill");
        assert_eq!(
            directory.file_name().and_then(|name| name.to_str()),
            Some("excel")
        );
    }

    #[test]
    fn invocation_schema_references_the_adjacent_catalog_without_duplicating_it() {
        let schema = invocation_id_schema();
        assert_eq!(schema.get("type").and_then(Value::as_str), Some("string"));
        assert!(schema
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains("canonical Skill id")));
        assert!(!schema
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains("Skill name")));
        assert!(schema.get("oneOf").is_none());
        assert!(schema
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("system reminder"));
    }

    #[test]
    fn every_built_in_skill_has_a_valid_runtime_execution_contract() {
        for skill in EMBEDDED_SKILLS.iter().filter(|skill| skill.user_facing) {
            let manifest_file = skill
                .files
                .iter()
                .find(|file| file.relative_path == "NOLA_MANIFEST.json")
                .expect("embedded manifest");
            let manifest: Value =
                serde_json::from_slice(manifest_file.contents).expect("manifest JSON");
            let contract = serde_json::from_value::<SkillExecutionContract>(
                manifest
                    .get("execution_contract")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .unwrap_or_else(|error| panic!("{} execution contract: {error}", skill.id));
            let instructions = skill.instruction.as_bytes();
            let digest = format!("{:x}", Sha256::digest(instructions));
            let mut runtime = SkillRuntimeState::default();
            runtime
                .apply(SkillRuntimeCommandEnvelope {
                    operation_id: format!("validate-contract:{}", skill.id),
                    command: SkillRuntimeCommand::Start {
                        run_id: format!("contract:{}", skill.id),
                        owner: SkillRunOwner {
                            session_id: "manifest-audit".to_string(),
                            parent_turn_id: "turn".to_string(),
                            child_turn_id: None,
                        },
                        descriptor: SkillDescriptor {
                            skill_id: skill.id.to_string(),
                            version: "manifest-audit".to_string(),
                            display_name: skill.title.to_string(),
                            instructions_ref: format!(
                                "skill://{}/manifest-audit/SKILL.md#sha256={digest}",
                                skill.id
                            ),
                            instructions_digest: digest,
                            required_tools: Vec::new(),
                            default_mode: Some(SkillExecutionMode::Inline),
                            execution_contract: contract,
                        },
                        mode: SkillExecutionMode::Inline,
                        budget: SkillRunBudget::default(),
                    },
                })
                .unwrap_or_else(|error| panic!("{} execution contract: {error:?}", skill.id));
        }
    }

    #[test]
    fn memory_skill_does_not_require_one_completion_shape_for_every_mode() {
        let skill = EMBEDDED_SKILLS
            .iter()
            .find(|skill| skill.id == "curate-knowledge-memory")
            .expect("memory Skill");
        let manifest_file = skill
            .files
            .iter()
            .find(|file| file.relative_path == "NOLA_MANIFEST.json")
            .expect("embedded manifest");
        let manifest: Value =
            serde_json::from_slice(manifest_file.contents).expect("manifest JSON");
        let contract = serde_json::from_value::<SkillExecutionContract>(
            manifest["execution_contract"].clone(),
        )
        .expect("memory execution contract");

        // Propose and recall do not create a memory record, while remember and
        // delete have different evidence. Their real memory operations and
        // canonical receipts must remain the execution record instead of a
        // synthetic, unconditional Skill checklist.
        assert!(contract.progress_steps.is_empty());
        assert!(contract.artifacts.is_empty());
        assert!(contract.verifiers.is_empty());
        assert!(skill
            .instruction
            .contains("actual memory tool calls and their canonical receipts"));
    }
}

pub(crate) fn split_payload(items: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut overrides = Vec::new();
    let mut uploaded = Vec::new();
    for item in items {
        let is_builtin = item.get("built_in").and_then(Value::as_bool) == Some(true)
            || item.get("source").and_then(Value::as_str) == Some("built_in");
        if !is_builtin {
            uploaded.push(normalize_uploaded_skill(item.clone()));
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(skill) = EMBEDDED_SKILLS.iter().find(|skill| skill.id == id) else {
            continue;
        };
        let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let removed = item
            .get("removed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut value = json!({"id": id, "enabled": enabled, "removed": removed});
        if let Some(content) = item.get("content").and_then(Value::as_str) {
            if content.trim_end() != skill.instruction.trim_end() {
                value["content"] = Value::String(content.to_string());
            }
        }
        if !enabled || removed || value.get("content").is_some() {
            overrides.push(value);
        }
    }
    (overrides, uploaded)
}

pub(crate) fn synchronize(overrides: &[Value], uploaded: &[Value]) -> Result<Vec<PathBuf>, String> {
    let roots = runtime_roots()?;
    let root = roots[0].clone();
    let custom_root = roots[1].clone();
    materialize_enabled_at(&root, overrides)?;
    materialize_custom_at(&custom_root, uploaded)?;
    Ok(roots)
}

pub(crate) fn runtime_roots() -> Result<Vec<PathBuf>, String> {
    Ok(vec![managed_root()?, managed_custom_root()?])
}

fn catalog_at(root: &Path, overrides: &[Value]) -> Vec<Value> {
    EMBEDDED_SKILLS
        .iter()
        .filter(|skill| skill.user_facing)
        .map(|skill| {
            let user_override = overrides
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(skill.id));
            let enabled = user_override
                .and_then(|item| item.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let removed = user_override
                .and_then(|item| item.get("removed"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let content = user_override
                .and_then(|item| item.get("content"))
                .and_then(Value::as_str)
                .unwrap_or(skill.instruction);
            let directory = root.join(skill.resource_dir);
            json!({
                "id": skill.id,
                "revision": user_override.and_then(|item| item.get("revision")).cloned().unwrap_or(Value::Null),
                "name": skill.title,
                "name_zh": localized_name(skill.id),
                "description": skill.description,
                "source": "built_in",
                "built_in": true,
                "enabled": enabled,
                "removed": removed,
                "content": content,
                "default_content": skill.instruction,
                "base_directory": directory.display().to_string(),
                "files": skill.files.iter().map(|file| file.relative_path).collect::<Vec<_>>(),
                "file_count": skill.files.len() + 1,
                "category_id": category_id(skill.id),
                "category_name": category_name(skill.id),
                "category_name_zh": localized_category_name(skill.id),
            })
        })
        .collect()
}

pub(crate) fn normalize_uploaded_skill(mut item: Value) -> Value {
    if let Some(object) = item.as_object_mut() {
        let id = stable_custom_skill_id(object);
        object.insert("id".to_string(), Value::String(id));
        object.insert("source".to_string(), Value::String("custom".to_string()));
        object.insert("built_in".to_string(), Value::Bool(false));
        object
            .entry("removed".to_string())
            .or_insert(Value::Bool(false));
    }
    item
}

fn stable_custom_skill_id(object: &serde_json::Map<String, Value>) -> String {
    let raw = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| object.get("name").and_then(Value::as_str).map(str::trim))
        .unwrap_or("skill");
    if valid_custom_skill_id(raw) {
        return raw.to_string();
    }
    let mut slug = String::new();
    let mut previous_separator = false;
    for character in raw.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            slug.push(character);
            previous_separator = false;
        } else if !previous_separator {
            slug.push('-');
            previous_separator = true;
        }
        if slug.len() >= 72 {
            break;
        }
    }
    let slug = slug.trim_matches(['.', '_', '-']).to_string();
    let slug = if slug.is_empty() {
        "skill".to_string()
    } else {
        slug
    };
    let seed = object
        .get("id")
        .or_else(|| object.get("name"))
        .or_else(|| object.get("content"))
        .map(Value::to_string)
        .unwrap_or_else(|| raw.to_string());
    format!("web-skill-{slug}-{:016x}", stable_hash(seed.as_bytes()))
}

fn valid_custom_skill_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf29ce484222325_u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

fn materialize_custom_at(root: &Path, uploaded: &[Value]) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "create custom Skill root {} failed: {error}",
            root.display()
        )
    })?;
    let normalized = uploaded
        .iter()
        .cloned()
        .map(normalize_uploaded_skill)
        .filter(|item| {
            crate::resource_access::allows(
                item,
                nola_core::agent::resource_access::AgentResourceKind::Skill,
                nola_core::agent::resource_access::AgentResourcePermission::ReadContent,
            )
        })
        .filter(|item| {
            item.get("enabled").and_then(Value::as_bool) != Some(false)
                && item.get("removed").and_then(Value::as_bool) != Some(true)
                && item
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| !content.trim().is_empty())
        })
        .collect::<Vec<_>>();
    let desired = normalized
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<HashSet<_>>();

    for entry in fs::read_dir(root)
        .map_err(|error| format!("read custom Skill root {} failed: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("read custom Skill entry failed: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() && !desired.contains(&name) {
            fs::remove_dir_all(entry.path()).map_err(|error| {
                format!(
                    "remove disabled custom Skill {} failed: {error}",
                    entry.path().display()
                )
            })?;
        }
    }

    for item in normalized {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let target = root.join(id);
        let staging = root.join(format!(".{id}.staging-{}", std::process::id()));
        let backup = root.join(format!(".{id}.backup-{}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|error| {
                format!(
                    "clear custom Skill staging {} failed: {error}",
                    staging.display()
                )
            })?;
        }
        fs::create_dir_all(&staging).map_err(|error| {
            format!(
                "create custom Skill staging {} failed: {error}",
                staging.display()
            )
        })?;
        fs::write(staging.join("SKILL.md"), content.as_bytes()).map_err(|error| {
            format!(
                "write custom Skill instructions {} failed: {error}",
                staging.display()
            )
        })?;
        let manifest = json!({
            "id": id,
            "name": item.get("name").cloned().unwrap_or_else(|| Value::String(id.to_string())),
            "description": item.get("description").cloned().unwrap_or(Value::Null),
            "source": "custom",
        });
        fs::write(
            staging.join("NOLA_MANIFEST.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| format!("encode custom Skill manifest failed: {error}"))?,
        )
        .map_err(|error| {
            format!(
                "write custom Skill manifest {} failed: {error}",
                staging.display()
            )
        })?;
        if backup.exists() {
            fs::remove_dir_all(&backup).map_err(|error| {
                format!(
                    "clear custom Skill backup {} failed: {error}",
                    backup.display()
                )
            })?;
        }
        if target.exists() {
            fs::rename(&target, &backup).map_err(|error| {
                format!(
                    "stage existing custom Skill {} failed: {error}",
                    target.display()
                )
            })?;
        }
        if let Err(error) = fs::rename(&staging, &target) {
            if backup.exists() {
                let _ = fs::rename(&backup, &target);
            }
            return Err(format!(
                "activate custom Skill {} failed: {error}",
                target.display()
            ));
        }
        if backup.exists() {
            fs::remove_dir_all(&backup).map_err(|error| {
                format!(
                    "remove custom Skill backup {} failed: {error}",
                    backup.display()
                )
            })?;
        }
    }
    Ok(())
}

fn materialize_enabled_at(root: &Path, overrides: &[Value]) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "create built-in Skill root {} failed: {error}",
            root.display()
        )
    })?;
    for skill in EMBEDDED_SKILLS {
        if !skill.user_facing {
            let directory = root.join(skill.resource_dir);
            materialize_embedded_skill(&directory, skill, skill.instruction)?;
            continue;
        }
        let user_override = overrides
            .iter()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(skill.id));
        let enabled = user_override
            .and_then(|item| item.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let removed = user_override
            .and_then(|item| item.get("removed"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let directory = root.join(skill.resource_dir);
        if !enabled || removed {
            if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| {
                    format!(
                        "remove disabled Skill {} failed: {error}",
                        directory.display()
                    )
                })?;
            }
            continue;
        }
        let content = user_override
            .and_then(|item| item.get("content"))
            .and_then(Value::as_str)
            .unwrap_or(skill.instruction);
        materialize_embedded_skill(&directory, skill, content)?;
    }
    Ok(())
}

fn materialize_embedded_skill(
    directory: &Path,
    skill: &EmbeddedSkill,
    instructions: &str,
) -> Result<(), String> {
    write_if_changed(&directory.join("SKILL.md"), instructions.as_bytes())?;
    for file in skill.files {
        if !safe_relative_path(file.relative_path) {
            return Err(format!(
                "unsafe embedded Skill path: {}",
                file.relative_path
            ));
        }
        let path = directory.join(file.relative_path);
        write_if_changed(&path, file.contents)?;
        #[cfg(unix)]
        if file.relative_path.starts_with("scripts/") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|error| {
                format!(
                    "mark Skill script executable {} failed: {error}",
                    path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<(), String> {
    if fs::read(path).ok().as_deref() == Some(contents) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create Skill directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, contents)
        .map_err(|error| format!("write Skill file {} failed: {error}", path.display()))
}

fn safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn localized_name(id: &str) -> &'static str {
    match id {
        "word-document-authoring" => "Word 文档创作",
        "excel-workbook-authoring" => "Excel 工作簿创作",
        "powerpoint-presentation-authoring" => "PowerPoint 演示文稿创作",
        "work-with-pdf" => "PDF 处理",
        "collaborate-in-figma" => "Figma 协作",
        "analyze-data" => "数据分析",
        "conduct-deep-research" => "深度研究",
        "review-documents" => "文档审阅",
        "extract-image-text" => "图片文字提取",
        "assist-meetings" => "会议助手",
        "plan-schedule-tasks" => "日程与任务规划",
        "curate-knowledge-memory" => "知识与记忆管理",
        "organize-files" => "文件整理",
        "design-automations" => "自动化设计",
        "assist-email" => "邮件助手",
        "nature-reader" => "科研论文精读",
        "nature-writing" => "科研论文写作",
        "nature-polishing" => "学术英文润色",
        "nature-reviewer" => "科研论文模拟审稿",
        "nature-response" => "审稿意见回复",
        "nature-statistics" => "统计报告审查",
        "nature-ref-verifier" => "学术参考文献核验",
        "nature-paper2ppt" => "论文转演示文稿",
        "install-open-source-model" => "开源模型安装",
        _ => "内置 Skill",
    }
}

fn category_id(id: &str) -> &'static str {
    match id {
        "word-document-authoring" | "work-with-pdf" | "review-documents" | "extract-image-text" => {
            "documents"
        }
        "analyze-data" | "excel-workbook-authoring" | "nature-statistics" => "data-analysis",
        "powerpoint-presentation-authoring" | "collaborate-in-figma" | "nature-paper2ppt" => {
            "design-presentations"
        }
        "assist-email" | "assist-meetings" => "communication",
        "plan-schedule-tasks" | "design-automations" => "planning-automation",
        "organize-files" | "install-open-source-model" => "workspace",
        _ => "knowledge-research",
    }
}

fn category_name(id: &str) -> &'static str {
    match category_id(id) {
        "documents" => "Documents",
        "data-analysis" => "Data & Analysis",
        "design-presentations" => "Design & Presentations",
        "communication" => "Communication",
        "planning-automation" => "Planning & Automation",
        "workspace" => "Workspace",
        _ => "Knowledge & Research",
    }
}

fn localized_category_name(id: &str) -> &'static str {
    match category_id(id) {
        "documents" => "文档处理",
        "data-analysis" => "数据与分析",
        "design-presentations" => "设计与演示",
        "communication" => "沟通协作",
        "planning-automation" => "计划与自动化",
        "workspace" => "工作区管理",
        _ => "知识与研究",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nola-skill-{name}-{}", std::process::id()))
    }

    #[test]
    fn shared_catalog_contains_all_product_skills() {
        assert_eq!(EMBEDDED_SKILLS.len(), 25);
        assert_eq!(catalog_at(&temporary_root("catalog"), &[]).len(), 24);
        assert_eq!(merged_catalog(&[], &[]).unwrap().len(), 24, "local built-in selections must survive the API ACL projection");
        assert!(EMBEDDED_SKILLS
            .iter()
            .any(|skill| skill.id == "word-document-authoring"));
        assert!(EMBEDDED_SKILLS
            .iter()
            .any(|skill| skill.id == "analyze-data"));
        assert!(EMBEDDED_SKILLS
            .iter()
            .any(|skill| skill.id == "nature-reader"));
        assert!(EMBEDDED_SKILLS
            .iter()
            .any(|skill| skill.id == "install-open-source-model"));
        let pdf = EMBEDDED_SKILLS
            .iter()
            .find(|skill| skill.id == "work-with-pdf")
            .expect("PDF Skill");
        assert!(pdf.instruction.contains("as the final tool call"));
        assert!(pdf
            .instruction
            .contains("After that receipt, stop calling tools"));
        let data_analysis = EMBEDDED_SKILLS
            .iter()
            .find(|skill| skill.id == "analyze-data")
            .expect("data analysis Skill");
        assert!(data_analysis
            .instruction
            .contains("Custom Markdown analysis contract"));
        assert!(data_analysis
            .instruction
            .contains("supporting deliverable, not a temporary helper"));
        let ocr = EMBEDDED_SKILLS
            .iter()
            .find(|skill| skill.id == "extract-image-text")
            .expect("OCR Skill");
        assert!(ocr.instruction.contains("--text-output"));
        assert!(ocr.instruction.contains("text_output_validated"));
        assert!(EMBEDDED_SKILLS
            .iter()
            .any(|skill| skill.id == "nature-shared" && !skill.user_facing));
        assert!(EMBEDDED_SKILLS
            .iter()
            .filter(|skill| skill.user_facing)
            .all(|skill| {
                skill
                    .files
                    .iter()
                    .any(|file| file.relative_path == "NOLA_MANIFEST.json")
            }));
        assert!(EMBEDDED_SKILLS.iter().all(|skill| {
            skill
                .files
                .iter()
                .all(|file| !file.relative_path.ends_with(".pyc"))
        }));
        assert_eq!(category_id("word-document-authoring"), "documents");
        assert_eq!(category_id("nature-statistics"), "data-analysis");
        assert_eq!(category_id("nature-paper2ppt"), "design-presentations");
        assert_eq!(category_id("nature-reader"), "knowledge-research");
        assert_eq!(category_id("install-open-source-model"), "workspace");
    }

    #[test]
    fn materialization_writes_scripts_and_removes_disabled_skills() {
        let root = temporary_root("materialize");
        let _ = fs::remove_dir_all(&root);
        materialize_enabled_at(&root, &[]).unwrap();
        assert!(root.join("word/scripts/create_document.py").is_file());
        materialize_enabled_at(
            &root,
            &[json!({"id":"word-document-authoring", "enabled":false})],
        )
        .unwrap();
        assert!(!root.join("word").exists());
        assert!(root.join("analyze-data/SKILL.md").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_payload_persists_only_changed_builtin_state() {
        let items = catalog_at(Path::new("/tmp/nola-skills"), &[]);
        let (overrides, uploaded) = split_payload(&items);
        assert!(overrides.is_empty());
        assert!(uploaded.is_empty());
        let (overrides, _) = split_payload(&[json!({
            "id":"analyze-data", "source":"built_in", "enabled":false, "removed":false
        })]);
        assert_eq!(
            overrides,
            vec![json!({"id":"analyze-data", "enabled":false, "removed":false})]
        );
        let mut edited = catalog_at(Path::new("/tmp/nola-skills"), &[json!({
            "id":"analyze-data", "enabled":false, "content":"custom instructions"
        })]).into_iter().find(|item| item["id"] == "analyze-data").unwrap();
        assert_eq!(edited["built_in"], true);
        edited["content"] = edited["default_content"].clone();
        let (restored, uploaded) = split_payload(&[edited]);
        assert!(uploaded.is_empty());
        assert_eq!(restored, vec![json!({"id":"analyze-data", "enabled":false, "removed":false})],
            "Restoring instructions clears only the content override, without enabling the skill");
    }

    #[test]
    fn custom_skill_catalog_is_metadata_only_and_materialization_tracks_enabled_state() {
        let root = temporary_root("custom-materialize");
        let _ = fs::remove_dir_all(&root);
        let custom = json!({
            "id": "Quarterly Report / Internal",
            "name": "Quarterly report",
            "description": "Build the quarterly report",
            "content": "# Private instructions\nNever place this body in discovery context.",
            "enabled": true
        });
        let normalized = normalize_uploaded_skill(custom.clone());
        let id = normalized.get("id").and_then(Value::as_str).unwrap();
        assert!(valid_custom_skill_id(id));
        assert_eq!(normalize_uploaded_skill(normalized.clone())["id"], id);
        let owned = crate::resource_access::restore_local_user_resource_identity(
            &custom,
            nola_core::agent::resource_access::AgentResourceKind::Skill,
        )
        .unwrap();
        materialize_custom_at(&root, &[owned]).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(id).join("SKILL.md")).unwrap(),
            "# Private instructions\nNever place this body in discovery context."
        );
        let disabled = crate::resource_access::restore_local_user_resource_identity(
            &json!({"id": id, "content": "ignored", "enabled": false}),
            nola_core::agent::resource_access::AgentResourceKind::Skill,
        )
        .unwrap();
        materialize_custom_at(&root, &[disabled]).unwrap();
        assert!(!root.join(id).exists());
        let _ = fs::remove_dir_all(root);
    }
}
