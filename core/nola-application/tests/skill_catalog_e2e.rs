use nola_application::{
    load_skill_catalog, register_skill, set_skill_enabled, SkillInput, SkillSource,
};
use nola_agent_engine::agent::capability::{CapabilityToolReference, SkillExecutionMode};
use nola_store::NolaStore;
use tempfile::tempdir;

fn skill(id: &str, enabled: bool) -> SkillInput {
    SkillInput {
        id: id.into(),
        source: SkillSource::BuiltIn,
        version: "2026.08.17".into(),
        display_name: "Document Authoring".into(),
        summary: "Create a verified document".into(),
        category: "documents".into(),
        instructions_ref: format!("nola-skill://{id}/2026.08.17"),
        instructions_digest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            .into(),
        instructions_text: "hello".into(),
        required_tools: vec![CapabilityToolReference::Builtin {
            tool_name: "write_file".into(),
        }],
        default_mode: Some(SkillExecutionMode::Fork),
        enabled,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[test]
fn application_projects_only_enabled_versioned_skills_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("skill-catalog.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let mut skills = store.skill_repository();
        register_skill(&mut skills, skill("word", true)).unwrap();
        register_skill(&mut skills, skill("disabled", false)).unwrap();
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let catalog = load_skill_catalog(&reopened.skill_repository()).unwrap();
    assert_eq!(catalog.skills.len(), 2);
    assert_eq!(catalog.descriptors.len(), 1);
    let descriptor = &catalog.descriptors[0];
    assert_eq!(descriptor.skill_id, "word");
    assert_eq!(descriptor.default_mode, Some(SkillExecutionMode::Fork));
    assert_eq!(
        descriptor.required_tools,
        vec![CapabilityToolReference::Builtin {
            tool_name: "write_file".into()
        }]
    );

    assert!(set_skill_enabled(&mut reopened.skill_repository(), "disabled", true, 2).unwrap());
    let catalog = load_skill_catalog(&reopened.skill_repository()).unwrap();
    assert_eq!(catalog.descriptors.len(), 2);
}
