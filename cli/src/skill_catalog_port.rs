use crate::application_port::CliApplicationPort;
use nola_application::{
    ApplicationCommand, Skill, SkillCatalogProjection, SkillCommand, SkillInput, SkillSource,
};
use nola_core::agent::capability::{CapabilityToolReference, SkillExecutionMode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// Embedded packages supply defaults; Core owns instruction selection and activation.
pub fn load_web_builtin_skills() -> Result<Vec<Value>, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    load_web_builtin_skills_with(&mut application)
}

pub(crate) fn load_web_builtin_skills_with(application: &mut CliApplicationPort) -> Result<Vec<Value>, String> {
    let mut defaults = crate::builtin_skills::catalog(&[])?;
    let now = unix_time_ms()?;
    let inputs = defaults.iter().map(|item| {
        let id = item["id"].as_str().ok_or("Built-in Skill ID missing")?.to_owned();
        let text = item["content"].as_str().ok_or("Built-in instructions missing")?.to_owned();
        let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
        Ok(SkillInput { id: id.clone(), source: SkillSource::BuiltIn, version: digest.clone(),
            display_name: item["name"].as_str().unwrap_or(&id).into(),
            summary: item["description"].as_str().unwrap_or("").into(),
            category: item["category_id"].as_str().unwrap_or("").into(),
            instructions_ref: format!("nola://skills/{id}/{digest}"), instructions_digest: digest,
            instructions_text: text, required_tools: Vec::new(), default_mode: None,
            enabled: true, created_at_ms: now, updated_at_ms: now })
    }).collect::<Result<Vec<_>, String>>()?;
    let editors: Vec<Value> = application.execute(ApplicationCommand::Skill {
        command: SkillCommand::SynchronizeBuiltIns { inputs },
    })?;
    for item in &mut defaults {
        let editor = editors.iter().find(|editor| editor["record"]["id"] == item["id"])
            .ok_or("Built-in Skill editor missing")?;
        item["revision"] = editor["revision"].clone();
        item["enabled"] = editor["record"]["enabled"].clone();
        item["content"] = editor["record"]["instructions_text"].clone();
    }
    Ok(defaults)
}

pub fn load_web_uploaded_skills() -> Result<Vec<Value>, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    load_web_uploaded_skills_with(&mut application)
}

pub(crate) fn load_web_uploaded_skills_with(
    application: &mut CliApplicationPort,
) -> Result<Vec<Value>, String> {
    let editors: Vec<Value> = application.execute(ApplicationCommand::Skill {
        command: SkillCommand::ListEditors,
    })?;
    editors
        .into_iter()
        .filter(|editor| editor["record"]["source"] == "user")
        .map(|editor| {
            let skill: Skill = serde_json::from_value(editor["record"].clone())
                .map_err(|error| error.to_string())?;
            let mut value = json!({
                "id": skill.id,
                "revision": editor["revision"],
                "name": skill.display_name,
                "description": skill.summary,
                "category_id": skill.category,
                "source": "custom",
                "built_in": false,
                "enabled": skill.enabled,
                "removed": false,
                "version": skill.version,
                "content": skill.instructions_text,
                "instructions_ref": skill.instructions_ref,
                "instructions_digest": skill.instructions_digest,
            });
            if !skill.required_tools.is_empty() {
                value["required_tools"] = serde_json::to_value(&skill.required_tools)
                    .map_err(|error| format!("encode Skill tools failed: {error}"))?;
            }
            if let Some(default_mode) = skill.default_mode {
                value["default_mode"] = serde_json::to_value(default_mode)
                    .map_err(|error| format!("encode Skill mode failed: {error}"))?;
            }
            crate::resource_access::restore_local_user_resource_identity(
                &value,
                nola_core::agent::resource_access::AgentResourceKind::Skill,
            )
        })
        .collect()
}

pub fn save_web_uploaded_skills(expected: &[Value], skills: &[Value]) -> Result<(), String> {
    let mut application = CliApplicationPort::persistent_default()?;
    save_web_uploaded_skills_with(&mut application, expected, skills)
}

pub(crate) fn save_web_uploaded_skills_with(
    application: &mut CliApplicationPort,
    expected: &[Value],
    skills: &[Value],
) -> Result<(), String> {
    let mut revisions = HashMap::new();
    for item in expected {
        let id = item["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("Skill baseline requires an ID")?;
        let revision = item["revision"]
            .as_i64()
            .filter(|revision| *revision > 0)
            .ok_or("Skill baseline requires a positive revision")?;
        if revisions.insert(id.to_owned(), revision).is_some() {
            return Err("Duplicate Skill baseline ID".into());
        }
    }
    let (builtins, custom): (Vec<_>, Vec<_>) = skills.iter().cloned().partition(|item| item["built_in"] == true || item["source"] == "built_in");
    let mut commands = Vec::new();
    for item in builtins {
        let id = item["id"].as_str().ok_or("Built-in Skill ID missing")?.to_owned();
        let expected_revision = revisions.remove(&id).ok_or("Built-in Skill baseline missing")?;
        commands.push(SkillCommand::ConfigureBuiltIn {
            id, expected_revision,
            instructions_text: item["content"].as_str().ok_or("Built-in Skill instructions missing")?.into(),
            enabled: item["enabled"].as_bool().unwrap_or(true) && item["removed"] != true,
            updated_at_ms: unix_time_ms()?,
        });
    }
    // A partial custom edit cannot delete a built-in Skill omitted from its payload.
    let editors: Vec<Value> = application.execute(ApplicationCommand::Skill { command: SkillCommand::ListEditors })?;
    for editor in editors.iter().filter(|editor| editor["record"]["source"] == "built_in") {
        if let Some(id) = editor["record"]["id"].as_str() { revisions.remove(id); }
    }
    let normalized = custom
        .iter()
        .cloned()
        .map(crate::builtin_skills::normalize_uploaded_skill)
        .collect::<Vec<_>>();
    let existing_projection = load_web_uploaded_skills_with(application)?
        .iter()
        .map(|value| {
            crate::resource_access::restore_local_user_resource_identity(
                value,
                nola_core::agent::resource_access::AgentResourceKind::Skill,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let reconciled = crate::resource_access::reconcile_owned_resources(
        &existing_projection,
        &normalized,
        nola_core::agent::resource_access::AgentResourceKind::Skill,
    )?;
    validate_skills(&reconciled)?;

    let catalog: SkillCatalogProjection = application.execute(ApplicationCommand::Skill {
        command: SkillCommand::LoadCatalog,
    })?;
    let existing = catalog
        .skills
        .into_iter()
        .filter(|skill| skill.source == SkillSource::User)
        .map(|skill| (skill.id.clone(), skill))
        .collect::<HashMap<_, _>>();
    let now_ms = unix_time_ms()?;
    for (index, skill) in reconciled.iter().enumerate() {
        let id = web_skill_id(skill, index);
        let expected_revision = revisions.remove(&id);
        let instructions = skill
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let digest = format!("{:x}", Sha256::digest(instructions.as_bytes()));
        let version = format!("content-{}", &digest[..16]);
        let display_name = skill
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Skill")
            .trim()
            .to_string();
        let summary = skill
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let category = skill
            .get("category_id")
            .and_then(Value::as_str)
            .unwrap_or("custom")
            .trim()
            .to_string();
        let required_tools = skill
            .get("required_tools")
            .cloned()
            .map(serde_json::from_value::<Vec<CapabilityToolReference>>)
            .transpose()
            .map_err(|error| format!("decode required tools for Skill {id} failed: {error}"))?
            .unwrap_or_default();
        let default_mode = skill
            .get("default_mode")
            .and_then(Value::as_str)
            .map(|mode| match mode {
                "inline" => Ok(SkillExecutionMode::Inline),
                "fork" => Ok(SkillExecutionMode::Fork),
                other => Err(format!("unsupported Skill mode: {other}")),
            })
            .transpose()?;
        let enabled = skill
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && skill.get("removed").and_then(Value::as_bool) != Some(true);

        let input = SkillInput {
            id: id.clone(),
            source: SkillSource::User,
            version: version.clone(),
            display_name,
            summary,
            category,
            instructions_ref: format!("nola://skills/{id}/{version}"),
            instructions_digest: digest,
            instructions_text: instructions,
            required_tools,
            default_mode,
            enabled,
            created_at_ms: existing
                .get(&id)
                .map(|value| value.created_at_ms)
                .unwrap_or(now_ms),
            updated_at_ms: now_ms,
        };
        if existing.get(&id).is_some_and(|current| {
            expected_revision.is_some()
                && current.display_name == input.display_name
                && current.summary == input.summary
                && current.category == input.category
                && current.instructions_text == input.instructions_text
                && current.required_tools == input.required_tools
                && current.default_mode == input.default_mode
                && current.enabled == input.enabled
        }) {
            continue;
        }
        commands.push(match expected_revision {
            Some(expected_revision) => SkillCommand::ConfigureUser {
                input,
                expected_revision,
            },
            None => SkillCommand::CreateUser { input },
        });
    }
    for (id, expected_revision) in revisions {
        commands.push(SkillCommand::DeleteUserChecked {
            id,
            expected_revision,
            deleted_at_ms: now_ms,
        });
    }
    if !commands.is_empty() {
        let _: Value = application.execute(ApplicationCommand::Skill {
            command: SkillCommand::ApplyChanges { commands },
        })?;
    }
    Ok(())
}

fn web_skill_id(skill: &Value, index: usize) -> String {
    skill
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("custom-skill-{index}"))
}

pub(crate) fn validate_skills(skills: &[Value]) -> Result<(), String> {
    if skills.len() > 512 {
        return Err("uploaded skills are limited to 512 entries".to_string());
    }
    let mut ids = HashSet::new();
    for (index, skill) in skills.iter().enumerate() {
        let id = web_skill_id(skill, index);
        if id.chars().count() > 256 {
            return Err(format!(
                "uploaded skill ID at index {index} exceeds 256 characters"
            ));
        }
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate uploaded skill ID: {id}"));
        }
        for key in ["name", "description", "category_id", "content"] {
            if skill
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!(
                    "uploaded skill at index {index} requires canonical field {key}"
                ));
            }
        }
        if skill
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.chars().count() > 200_000)
        {
            return Err(format!(
                "uploaded skill content at index {index} exceeds 200000 characters"
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
