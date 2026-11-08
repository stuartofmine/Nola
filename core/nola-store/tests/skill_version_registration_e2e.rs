// SPDX-License-Identifier: Apache-2.0
use nola_store::{NolaStore, SkillSourceKind, UpsertSkillInput};

fn package(version: &str) -> UpsertSkillInput {
    UpsertSkillInput {
        id: "word".into(),
        source: SkillSourceKind::BuiltIn,
        version: version.into(),
        display_name: "Word".into(),
        summary: "Document authoring".into(),
        category: "documents".into(),
        instructions_ref: format!("skill://word/{version}/SKILL.md"),
        instructions_digest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            .into(),
        instructions_text: "hello".into(),
        required_tools_json: None,
        default_mode: None,
        enabled: true,
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

#[test]
fn registering_version_does_not_select_it_or_change_editor_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("versions.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.upsert_skill(&package("v1")).unwrap();
    store.set_skill_enabled("word", false, 3).unwrap();
    let baseline = store.skill_editor_snapshot("word").unwrap();
    let next = package("v2");
    store.register_skill_version(&next).unwrap();
    store.register_skill_version(&next).unwrap();
    assert_eq!(store.skill_editor_snapshot("word").unwrap(), baseline);
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let mut conflicting = next.clone();
    conflicting.instructions_ref = "skill://word/different/SKILL.md".into();
    assert!(store.register_skill_version(&conflicting).is_err());
    assert!(store.upsert_skill(&conflicting).is_err());
    assert_eq!(store.skill_editor_snapshot("word").unwrap(), baseline);
    let selected = store.upsert_skill(&next).unwrap();
    assert_eq!(selected.version, "v2");
    assert!(!selected.enabled);
}

#[test]
fn registration_rejects_missing_identity_wrong_source_and_invalid_digest() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("invalid.db")).unwrap();
    assert!(store.register_skill_version(&package("v1")).is_err());
    store.upsert_skill(&package("v1")).unwrap();
    let baseline = store.skill_editor_snapshot("word").unwrap();
    let mut wrong_source = package("v2");
    wrong_source.source = SkillSourceKind::User;
    assert!(store.register_skill_version(&wrong_source).is_err());
    let mut invalid = package("v2");
    invalid.instructions_digest = "invalid".into();
    assert!(store.register_skill_version(&invalid).is_err());
    assert_eq!(store.skill_editor_snapshot("word").unwrap(), baseline);
}
