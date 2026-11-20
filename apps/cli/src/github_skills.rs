// SPDX-License-Identifier: Apache-2.0
//! Register a prepared GitHub package in this Host's existing Skill settings.
use crate::application_port::CliApplicationPort;
use nola_application::{ApplicationCommand, SkillCommand, SkillInput, SkillSource};
pub(crate) use nola_skill_host::{find, resource_base, source_url, RESOURCE_TREE};
use serde_json::{json, Value};
use std::path::PathBuf;

fn package_store() -> Result<PathBuf, String> {
    Ok(crate::platform_paths::application_database_path()?.with_extension("skill-packages"))
}

pub(crate) fn package_for_reference(reference: &str) -> Result<Option<(PathBuf, String)>, String> {
    nola_skill_host::package_for_reference(&package_store()?, reference)
}

pub(crate) fn install(input: &str) -> Result<Value, String> {
    nola_skill_host::prepare_with(&package_store()?, input, |prepared| {
        let nola_skill_host::PreparedPackage {
            id,
            description,
            content: text,
            version,
            instructions_ref: reference,
            source_url: origin,
            ..
        } = prepared;
        if crate::builtin_skills::user_facing_ids().contains(&id.as_str()) {
            return Err(format!("{id} is already a built-in Skill"));
        }
        let mut app = CliApplicationPort::persistent_default()?;
        let now = chrono::Utc::now().timestamp_millis();
        let current: Value = app.execute(ApplicationCommand::Skill {
            command: SkillCommand::GetEditor { id: id.clone() },
        })?;
        if !current["record"]["deleted_at_ms"].is_null() {
            return Err(format!("The previously deleted Skill ID {id} is reserved by its history. To install a separate Skill, choose a package with a different name."));
        }
        if !current.is_null()
            && (current["record"]["source"] != "user"
                || current["record"]["instructions_ref"] != reference)
        {
            return Err(format!(
                "Skill ID {id} already belongs to another installation or version"
            ));
        }
        if current.is_null() {
            let input = SkillInput {
                id: id.clone(),
                source: SkillSource::User,
                version,
                display_name: id.clone(),
                summary: description,
                category: "custom".into(),
                instructions_ref: reference,
                instructions_text: text,
                required_tools: Vec::new(),
                default_mode: None,
                enabled: true,
                created_at_ms: now,
                updated_at_ms: now,
            };
            let _: Value = app.execute(ApplicationCommand::Skill {
                command: SkillCommand::CreateUser { input },
            })?;
        } else if current["record"]["enabled"] != true {
            let _: Value = app.execute(ApplicationCommand::Skill {
                command: SkillCommand::SetEnabledChecked {
                    id: id.clone(),
                    expected_revision: current["revision"]
                        .as_i64()
                        .ok_or("Missing Skill revision")?,
                    enabled: true,
                    updated_at_ms: now,
                },
            })?;
        }
        crate::builtin_skills::sync_registered_custom(&mut app, Some(&id))?;
        Ok(json!({"status":"installed","id":id,"enabled":true,"source_url":origin}))
    })
}

/// Editing instructions or their front matter must not rename a registered Skill.
pub(crate) fn registered_metadata(
    managed_root: &std::path::Path,
    directory: &std::path::Path,
) -> Option<Value> {
    if directory.parent()? != managed_root {
        return None;
    }
    let id = directory.file_name()?.to_str()?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("NOLA_MANIFEST.json")).ok()?).ok()?;
    if manifest["source"] != "custom"
        || manifest["id"] != id
        || manifest["source_url"].as_str().is_none()
    {
        return None;
    }
    Some(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_metadata_only_applies_to_the_owned_package_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("card");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("NOLA_MANIFEST.json"),
            r#"{"source":"custom","id":"card","source_url":"https://github.com/a/b"}"#,
        )
        .unwrap();
        assert_eq!(
            registered_metadata(tmp.path(), &directory).unwrap()["id"],
            "card"
        );
        assert!(registered_metadata(&tmp.path().join("elsewhere"), &directory).is_none());
    }
}
