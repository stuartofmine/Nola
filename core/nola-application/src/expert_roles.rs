// SPDX-License-Identifier: Apache-2.0
//! Immutable role starting points shared by every client. A tool pack is
//! guidance for the model, never a capability grant or name-based router.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertRoleCatalog {
    pub version: u32,
    pub features: BTreeMap<String, bool>,
    pub templates: Vec<ExpertRoleTemplate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertRoleTemplate {
    pub id: String,
    pub role_id: String,
    pub name: BTreeMap<String, String>,
    pub description: BTreeMap<String, String>,
    pub prompt: String,
    pub tool_pack: ExpertToolPack,
    pub skills: Vec<ExpertRoleSkill>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertToolPack {
    pub id: String,
    pub tools: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertRoleSkill {
    pub id: String,
    pub name: BTreeMap<String, String>,
    pub instructions: String,
}

pub fn catalog() -> &'static ExpertRoleCatalog {
    static CATALOG: OnceLock<ExpertRoleCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("expert_roles/catalog.json"))
            .expect("compiled expert role catalog must be valid")
    })
}
pub fn find(id: &str) -> Option<&'static ExpertRoleTemplate> {
    let id = uuid::Uuid::parse_str(id).ok()?;
    catalog()
        .templates
        .iter()
        .find(|role| uuid::Uuid::parse_str(&role.id).ok() == Some(id))
}

/// Lower the expert's explicit choice to the existing generation contract only
/// after its concrete model has been resolved. Inheritance never writes none.
pub(crate) fn apply_reasoning_preference(
    configuration: &mut serde_json::Value,
    model: &str,
) -> Result<(), String> {
    let effort = match configuration.get("expertReasoningEffort") {
        None | Some(serde_json::Value::Null) => return Ok(()),
        Some(value) => value.as_str().ok_or("invalid expert reasoning effort")?,
    };
    if effort == "automatic" {
        return Ok(());
    }
    if !nola_domain::generation::REASONING_EFFORT_VALUES.contains(&effort) {
        return Err("invalid expert reasoning effort".into());
    }
    let effort = effort.to_owned();
    let configuration = configuration
        .as_object_mut()
        .ok_or("invalid expert configuration")?;
    let overrides = configuration
        .entry("modelOverrides")
        .or_insert_with(|| serde_json::json!({}));
    if overrides.is_null() {
        *overrides = serde_json::json!({});
    }
    let overrides = overrides
        .as_object_mut()
        .ok_or("invalid expert model overrides")?;
    let selected = overrides
        .entry(model)
        .or_insert_with(|| serde_json::json!({}));
    if selected.is_null() {
        *selected = serde_json::json!({});
    }
    let selected = selected
        .as_object_mut()
        .ok_or("invalid expert selected model override")?;
    selected.remove("reasoningTier");
    selected.insert(
        "reasoningPreference".into(),
        serde_json::Value::String(effort),
    );
    Ok(())
}
impl ExpertRoleTemplate {
    /// Captured as the ordinary immutable prompt definition. Skill bodies are
    /// still loaded from the current authorized Skill catalog, not copied from
    /// untrusted client descriptions or silently enabled by selecting a role.
    pub fn instructions(&self) -> String {
        let mut body = format!("{}\n\nRespond in the user's language.\n\n## Role tool pack: {}\n{}\n\nThese are intended operations, not proof that handlers are available or authorized. Use this compact pack as the default working set for the role. Shared discovery, Skill bookkeeping, permissions and required task/workspace protocols are runtime support, not extra business tools to preload. Select actual schemas based on the whole task; discover additional capabilities only when the concrete deliverable requires them within this specialty. Do not load unrelated tool packs. If a different specialty is required, explain the scope and ask the user to select the appropriate expert. Do not force tools from words in the user's message. Honor resource policy and concrete permission checks. If an operation is unavailable, explain what can actually be done; never simulate its execution.\n\n## Associated Skills\n",
            self.prompt, self.tool_pack.id, self.tool_pack.tools.join(", "));
        for skill in &self.skills {
            body.push_str(&format!("- {}: {}\n", skill.id, skill.instructions));
        }
        body.push_str("Role selection is a professional perspective, not an instruction to begin its workflow on every message. Follow the current user's concrete intent; explanatory or conversational requests do not request artifact creation or execution. Load applicable Skills by their exact IDs from the active authorized catalog before following their instructions. Selection of this role neither installs nor enables a disabled Skill. If a required Skill or Host is unavailable, state that limitation and preserve the user's work. A missing image/deck tool is not a completed image/deck.\n");
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_ids_and_packages_are_complete_unique_and_stable() {
        let catalog = catalog();
        assert_eq!(catalog.version, 1);
        assert_eq!(catalog.features.get("reasoning_effort"), Some(&true));
        assert_eq!(catalog.features.get("main_agent_delegation"), Some(&true));
        assert!(crate::ApplicationCommand::AgentRoom {
            command: crate::agent_room::AgentRoomCommand::RoleTemplates
        }
        .is_read_only_query());
        assert_eq!(catalog.templates.len(), 7);
        let mut ids = std::collections::BTreeSet::new();
        for role in &catalog.templates {
            assert!(ids.insert(uuid::Uuid::parse_str(&role.id).unwrap()));
            assert!(
                !role.prompt.is_empty()
                    && !role.tool_pack.tools.is_empty()
                    && !role.skills.is_empty()
            );
            assert_eq!(find(&role.id).unwrap().role_id, role.role_id);
            for language in ["zh", "en", "de", "es", "fr", "ja", "ru"] {
                assert!(!role.name[language].is_empty());
            }
            assert!(role.tool_pack.tools.len() <= 5, "Default role packs must stay compact");
            assert_eq!(role.tool_pack.tools.iter().collect::<std::collections::BTreeSet<_>>().len(), role.tool_pack.tools.len());
            for protocol in ["Skill", "skill_update", "spawn_agent", "read_task", "wait_task", "code_mode_exec"] {
                assert!(!role.tool_pack.tools.iter().any(|tool| tool == protocol), "Runtime support is not a role business pack");
            }
            let body = role.instructions();
            assert!(body.contains("not proof") && body.contains("neither installs nor enables"));
            for skill in &role.skills {
                assert!(body.contains(&skill.id));
            }
        }
        let reviewer = catalog.templates.iter().find(|role| role.role_id == "code-reviewer").unwrap();
        assert!(!reviewer.tool_pack.tools.iter().any(|tool| matches!(tool.as_str(), "bash" | "write_file" | "edit_file")));
        for role_id in ["private-secretary", "visual-artist"] {
            let role = catalog.templates.iter().find(|role| role.role_id == role_id).unwrap();
            assert!(!role.tool_pack.tools.iter().any(|tool| matches!(tool.as_str(), "bash" | "write_file" | "edit_file" | "read_file")));
        }
        assert!(find("not-a-template").is_none());
    }

    #[test]
    fn effort_is_sparse_preserves_other_parameters_and_never_defaults_to_none() {
        let original = serde_json::json!({"modelOverrides":{"configured:one":{"temperature":0.2,"reasoningTier":"detailed"},"configured:other":{"topP":0.4}}});
        let mut value = original.clone();
        apply_reasoning_preference(&mut value, "configured:one").unwrap();
        assert_eq!(value, original);
        value["expertReasoningEffort"] = serde_json::json!("automatic");
        let inherited = value.clone();
        apply_reasoning_preference(&mut value, "configured:one").unwrap();
        assert_eq!(value, inherited);
        value["expertReasoningEffort"] = serde_json::json!("xhigh");
        apply_reasoning_preference(&mut value, "configured:one").unwrap();
        assert_eq!(
            value["modelOverrides"]["configured:one"]["reasoningPreference"],
            "xhigh"
        );
        assert!(value["modelOverrides"]["configured:one"]
            .get("reasoningTier")
            .is_none());
        assert_eq!(
            value["modelOverrides"]["configured:one"]["temperature"],
            0.2
        );
        assert_eq!(
            value["modelOverrides"]["configured:other"],
            original["modelOverrides"]["configured:other"]
        );
        assert!(nola_domain::persistent_agent::AgentDefaults::try_from(value).is_ok());
        for invalid in [
            serde_json::json!("ultra"),
            serde_json::json!(42),
            serde_json::json!(false),
        ] {
            assert!(nola_domain::persistent_agent::AgentDefaults::try_from(
                serde_json::json!({"expertReasoningEffort":invalid})
            )
            .is_err());
        }
    }

    #[test]
    fn every_role_skill_is_a_real_bundled_package() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/macos/Resources/NolaBuiltInSkills.bundle");
        let manifests = std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                std::fs::read_to_string(entry.path().join("NOLA_MANIFEST.json"))
                    .ok()
                    .map(|body| {
                        (
                            entry.path(),
                            serde_json::from_str::<serde_json::Value>(&body).unwrap(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        for role in &catalog().templates {
            for skill in &role.skills {
                let (path, _) = manifests
                    .iter()
                    .find(|(_, manifest)| manifest["id"] == skill.id)
                    .unwrap_or_else(|| panic!("Missing bundled Skill {}", skill.id));
                assert!(path.join("SKILL.md").is_file());
            }
        }
    }
}
