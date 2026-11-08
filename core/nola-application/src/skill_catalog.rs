// SPDX-License-Identifier: Apache-2.0
//! Product Skill catalog use cases and Host descriptor projection.

use nola_agent_engine::agent::capability::SkillDescriptor;
use serde::{Deserialize, Serialize};

use crate::{Skill, SkillInput};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCatalogProjection {
    pub skills: Vec<Skill>,
    pub descriptors: Vec<SkillDescriptor>,
}

pub fn register_skill(
    store: &mut impl nola_store_port::skill::SkillRepository,
    input: SkillInput,
) -> Result<Skill, String> {
    let input = nola_store_port::skill::UpsertSkillInput::try_from(input)?;
    let stored = store
        .upsert_skill(&input)
        .map_err(|error| format!("register Skill failed: {error}"))?;
    Skill::try_from(stored)
}

pub fn set_skill_enabled(
    store: &mut impl nola_store_port::skill::SkillRepository,
    id: &str,
    enabled: bool,
    updated_at_ms: i64,
) -> Result<bool, String> {
    store
        .set_skill_enabled(id, enabled, updated_at_ms)
        .map_err(|error| format!("update Skill availability failed: {error}"))
}

pub fn delete_user_skill(
    store: &mut impl nola_store_port::skill::SkillRepository,
    id: &str,
    deleted_at_ms: i64,
) -> Result<bool, String> {
    store
        .delete_user_skill(id, deleted_at_ms)
        .map_err(|error| format!("delete user Skill failed: {error}"))
}

/// Produces both product-facing records and exact Host descriptors.
/// Hosts materialize these references; clients do not infer Skill routing.
pub fn load_skill_catalog(
    store: &impl nola_store_port::skill::SkillRepository,
) -> Result<SkillCatalogProjection, String> {
    let skills = store
        .list_skills(false)
        .map_err(|error| format!("load Skill catalog failed: {error}"))?
        .into_iter()
        .map(Skill::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let descriptors = skills
        .iter()
        .filter(|skill| skill.enabled)
        .map(skill_descriptor)
        .collect::<Vec<_>>();
    Ok(SkillCatalogProjection {
        skills,
        descriptors,
    })
}

fn skill_descriptor(skill: &Skill) -> SkillDescriptor {
    SkillDescriptor {
        skill_id: skill.id.clone(),
        version: skill.version.clone(),
        display_name: skill.display_name.clone(),
        instructions_ref: skill.instructions_ref.clone(),
        instructions_digest: skill.instructions_digest.clone(),
        required_tools: skill.required_tools.clone(),
        default_mode: skill.default_mode,
        execution_contract: Default::default(),
    }
}
