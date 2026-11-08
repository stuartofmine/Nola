// SPDX-License-Identifier: Apache-2.0
//! Application lifecycle tested without SQLite or an Application Runtime.
use crate::data_source::*;
use nola_domain::data_source::DataSourceAccessPolicy;
use nola_store_port::data_source::*;

#[derive(Default)]
struct Repository {
    record: Option<StoredDataSourceRecord>,
    writes: usize,
    fail_reads: bool,
}

impl DataSourceReadRepository for Repository {
    type Error = String;
    fn data_source(&self, id: &str) -> Result<Option<StoredDataSourceRecord>, String> {
        if self.fail_reads {
            return Err("storage unavailable".into());
        }
        Ok(self.record.clone().filter(|source| source.id == id))
    }
    fn list_data_sources(&self) -> Result<Vec<StoredDataSourceRecord>, String> {
        panic!("unexpected list")
    }
    fn list_data_sources_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<(StoredDataSourceRecord, Option<bool>)>, String> {
        assert_eq!(conversation_id, "selected-conversation");
        if self.fail_reads {
            return Err("snapshot unavailable".into());
        }
        Ok(self
            .record
            .clone()
            .into_iter()
            .map(|source| (source, Some(false)))
            .collect())
    }
    fn data_source_override(&self, _: &str, _: &str) -> Result<Option<bool>, String> {
        panic!("unexpected override read")
    }
    fn list_data_source_usage(&self, _: usize) -> Result<Vec<StoredDataSourceUsage>, String> {
        panic!("unexpected usage read")
    }
}

impl DataSourceRepository for Repository {
    fn upsert_data_source(
        &mut self,
        record: &StoredDataSourceRecord,
    ) -> Result<StoredDataSourceRecord, String> {
        self.writes += 1;
        let mut saved = record.clone();
        saved.revision += 1;
        self.record = Some(saved.clone());
        Ok(saved)
    }
    fn delete_data_source(&mut self, _: &str, _: i64) -> Result<bool, String> {
        panic!("unexpected delete")
    }
    fn set_data_source_override(
        &mut self,
        _: &str,
        _: &str,
        _: Option<bool>,
        _: i64,
    ) -> Result<(), String> {
        panic!("unexpected override write")
    }
    fn append_data_source_usage(&mut self, _: &StoredDataSourceUsage) -> Result<(), String> {
        panic!("unexpected usage write")
    }
}

fn draft() -> DataSourceDraft {
    DataSourceDraft {
        id: "source".into(),
        name: "Documents".into(),
        kind: DataSourceKind::Directory,
        configuration: DataSourceConfiguration {
            connector_id: "directory".into(),
            location_kind: DataSourceLocationKind::LocalPath,
            location: "/documents".into(),
            parameters: Default::default(),
            secret_references: vec![],
        },
        access_policy: DataSourceAccessPolicy::ConversationOptIn,
        authorization: DataSourceAuthorization {
            state: DataSourceAuthorizationState::NotRequired,
            capability: None,
            observed_at_ms: 1,
        },
        freshness_policy: DataSourceFreshnessPolicy {
            maximum_age_minutes: 60,
            allow_stale_fallback: false,
        },
        capabilities: vec!["read".into()],
        privacy: "private".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[test]
fn lifecycle_guards_precede_repository_writes_without_sqlite() {
    let mut repository = Repository::default();
    let value = execute_data_source_command(
        &mut repository,
        DataSourceCommand::CreateDraft { input: draft() },
    )
    .unwrap();
    assert_eq!(value["revision"], 1);
    assert_eq!(value["default_enabled"], false);
    for expected_revision in [0, 1] {
        assert!(execute_data_source_command(
            &mut repository,
            DataSourceCommand::SetDefaultEnabled {
                id: "source".into(),
                expected_revision,
                enabled: true,
                updated_at_ms: 2,
            }
        )
        .is_err());
    }
    assert_eq!(repository.writes, 1);
    let pending = execute_data_source_command(
        &mut repository,
        DataSourceCommand::RequestValidation {
            id: "source".into(),
            expected_revision: 1,
            requested_at_ms: 2,
        },
    )
    .unwrap();
    assert_eq!(pending["revision"], 2);
    assert_eq!(pending["validation"]["status"], "pending");
    let token = pending["validation"]["validation_token"]
        .as_str()
        .unwrap()
        .to_string();
    for validation_token in ["stale".to_string(), token] {
        let result = execute_data_source_command(
            &mut repository,
            DataSourceCommand::RecordValidation {
                id: "source".into(),
                validation_token: validation_token.clone(),
                result: DataSourceValidationResult::Failed {
                    code: "offline".into(),
                    diagnostic: "unavailable".into(),
                },
                recorded_at_ms: 3,
            },
        );
        if validation_token == "stale" {
            assert!(result.is_err());
            assert_eq!(repository.writes, 2);
        } else {
            assert_eq!(result.unwrap()["lifecycle"], "failed");
            assert_eq!(repository.writes, 3);
        }
    }
}

#[test]
fn malformed_input_and_storage_failure_never_reach_a_write() {
    let mut repository = Repository {
        fail_reads: true,
        ..Default::default()
    };
    let mut invalid = draft();
    invalid.capabilities.clear();
    let error = execute_data_source_command(
        &mut repository,
        DataSourceCommand::CreateDraft { input: invalid },
    )
    .unwrap_err();
    assert!(error.contains("capabilities"));
    let error = execute_data_source_command(
        &mut repository,
        DataSourceCommand::CreateDraft { input: draft() },
    )
    .unwrap_err();
    assert!(error.contains("storage unavailable"));
    assert_eq!(repository.writes, 0);
}

#[test]
fn context_resolution_uses_batch_snapshot_and_propagates_read_failure() {
    let mut repository = Repository::default();
    execute_data_source_command(
        &mut repository,
        DataSourceCommand::CreateDraft { input: draft() },
    )
    .unwrap();
    let command = || DataSourceCommand::ResolveForConversation {
        conversation_id: "selected-conversation".into(),
        now_ms: 10,
    };
    // Individual list and override methods panic, so this checks the actual
    // use-case path rather than merely exercising a new unused port method.
    let value = execute_data_source_command(&mut repository, command()).unwrap();
    assert_eq!(value[0]["reason"], "disabled_for_conversation");
    assert_eq!(value[0]["enabled"], false);
    repository.fail_reads = true;
    let error = execute_data_source_command(&mut repository, command()).unwrap_err();
    assert!(error.contains("snapshot unavailable"));
    assert_eq!(repository.writes, 1);
}
