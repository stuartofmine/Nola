// SPDX-License-Identifier: Apache-2.0
use nola_application::persistent_agent::{self as agents, ConfigureAgent};
use nola_domain::persistent_agent::{AgentAvailability, AgentDefaults, AgentId};
use nola_store::NolaStore;
use serde_json::json;

fn command(id: &str, directory: Option<i64>, revision: Option<i64>) -> ConfigureAgent {
    ConfigureAgent {
        id: AgentId::try_from(id.to_string()).unwrap(),
        expected_directory_revision: directory,
        expected_agent_revision: revision,
        name: format!("Assistant {id}"),
        description: "Public research responsibilities".into(),
        defaults: AgentDefaults::try_from(json!({"language":"zh"})).unwrap(),
        availability: AgentAvailability::Active,
        updated_at_ms: 1,
    }
}

#[test]
fn directory_is_transactional_and_stale_edits_do_not_overwrite_other_agents() {
    let mut store = NolaStore::open(":memory:").unwrap();
    assert!(agents::configure(
        &mut store.product_setting_repository(),
        command("a", None, None)
    )
    .is_err());
    store.begin_write_unit_of_work().unwrap();
    let first = agents::configure(
        &mut store.product_setting_repository(),
        command("a", None, None),
    )
    .unwrap();
    assert_eq!(first.agents.len(), 1);
    store.rollback_write_unit_of_work().unwrap();
    assert!(agents::load(&store.product_setting_repository())
        .unwrap()
        .agents
        .is_empty());

    store.begin_write_unit_of_work().unwrap();
    let first = agents::configure(
        &mut store.product_setting_repository(),
        command("a", None, None),
    )
    .unwrap();
    let second = agents::configure(
        &mut store.product_setting_repository(),
        command("b", first.revision, None),
    )
    .unwrap();
    assert!(agents::configure(
        &mut store.product_setting_repository(),
        command("a", first.revision, Some(1))
    )
    .is_err());
    assert_eq!(
        agents::load(&store.product_setting_repository()).unwrap(),
        second
    );
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(
        agents::load(&store.product_setting_repository())
            .unwrap()
            .agents
            .len(),
        2
    );
}

#[test]
fn captured_configuration_is_independent_and_archival_preserves_directory() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    let first = agents::configure(
        &mut store.product_setting_repository(),
        command("a", None, None),
    )
    .unwrap();
    let id = AgentId::try_from("a".to_string()).unwrap();
    let captured = agents::capture_defaults(&store.product_setting_repository(), &id, 1).unwrap();
    let mut update = command("a", first.revision, Some(1));
    update.defaults = AgentDefaults::try_from(json!({"language":"en"})).unwrap();
    let second = agents::configure(&mut store.product_setting_repository(), update).unwrap();
    assert_eq!(captured.initial_configuration["language"], "zh");
    assert!(agents::capture_defaults(&store.product_setting_repository(), &id, 1).is_err());
    assert_eq!(
        agents::capture_defaults(&store.product_setting_repository(), &id, 2)
            .unwrap()
            .initial_configuration["language"],
        "en"
    );
    let mut archive = command("a", second.revision, Some(2));
    archive.availability = AgentAvailability::Archived;
    agents::configure(&mut store.product_setting_repository(), archive).unwrap();
    assert!(agents::capture_defaults(&store.product_setting_repository(), &id, 3).is_err());
    assert_eq!(
        agents::load(&store.product_setting_repository())
            .unwrap()
            .agents
            .len(),
        1
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn malformed_persisted_directory_is_rejected_without_repair() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let raw = r#"{"agents":[],"old_format":true}"#;
    store
        .put_product_setting(agents::DIRECTORY_KEY, raw, 1)
        .unwrap();
    assert!(agents::load(&store.product_setting_repository()).is_err());
    assert_eq!(
        store
            .product_setting(agents::DIRECTORY_KEY)
            .unwrap()
            .unwrap()
            .value_json,
        raw
    );
}

#[test]
fn committed_agents_survive_store_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("agents.sqlite");
    {
        let mut store = NolaStore::open(&path).unwrap();
        store.begin_write_unit_of_work().unwrap();
        agents::configure(
            &mut store.product_setting_repository(),
            command("a", None, None),
        )
        .unwrap();
        store.commit_write_unit_of_work().unwrap();
    }
    let mut reopened = NolaStore::open(&path).unwrap();
    let directory = agents::load(&reopened.product_setting_repository()).unwrap();
    assert_eq!(directory.revision, Some(1));
    assert_eq!(directory.agents[0].id().as_str(), "a");
    assert_eq!(
        directory.agents[0].defaults().conversation_configuration()["language"],
        "zh"
    );
}
