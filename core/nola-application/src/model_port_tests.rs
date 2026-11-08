// SPDX-License-Identifier: Apache-2.0
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Repository {
    profiles: BTreeMap<String, StoredModelProfile>,
    runtime: Option<StoredModelRuntime>,
    fail_sync: bool,
}
impl ModelRepository for Repository {
    type Error = String;
    fn reserve_model_probe(
        &mut self,
        _: &nola_domain::model_connection::ModelProbeAttempt,
    ) -> Result<(), Self::Error> {
        Err("covered by integration fixtures".into())
    }
    fn finish_model_probe(&mut self, _: &str, _: bool, _: i64) -> Result<(), Self::Error> {
        Err("covered by integration fixtures".into())
    }
    fn model_probe_attempt(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::model_connection::ModelProbeAttempt>, Self::Error> {
        Ok(None)
    }
    fn save_gateway_attempt(
        &mut self,
        _: &nola_domain::model_connection::GatewayAttemptRecord,
        _: Option<i64>,
    ) -> Result<nola_domain::model_connection::GatewayAttemptSnapshot, Self::Error> {
        Err("gateway writes covered by integration fixtures".into())
    }
    fn list_gateway_attempts(
        &self,
        _: usize,
        _: usize,
    ) -> Result<Vec<nola_domain::model_connection::GatewayAttemptSnapshot>, Self::Error> {
        Ok(vec![])
    }
    fn model_connection(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> {
        Ok(None)
    }
    fn list_model_connections(
        &self,
    ) -> Result<Vec<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> {
        Ok(vec![])
    }
    fn commit_model_connection(
        &mut self,
        _: &nola_domain::model_connection::ModelConnectionDocument,
        _: Option<i64>,
        _: &[StoredModelProfile],
        _: &BTreeMap<String, i64>,
        _: i64,
    ) -> Result<nola_domain::model_connection::ModelConnectionSnapshot, Self::Error> {
        Err("connection writes are covered by integration fixtures".into())
    }
    fn model_account(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> {
        Ok(None)
    }
    fn list_model_accounts(
        &self,
    ) -> Result<Vec<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> {
        Ok(vec![])
    }
    fn save_model_account(
        &mut self,
        _: &nola_domain::model_connection::ModelAccount,
        _: Option<i64>,
        _: i64,
    ) -> Result<nola_domain::model_connection::ModelAccountSnapshot, Self::Error> {
        Err("account writes are covered by integration fixtures".into())
    }
    fn configure_model_endpoint(
        &mut self,
        _: &str,
        _: &BTreeMap<String, i64>,
        _: &[StoredModelProfile],
    ) -> Result<(), String> {
        Err("endpoint configuration is not exercised by this fixture".into())
    }
    fn configure_model_parameters(&mut self, input: &StoredModelProfile) -> Result<(), String> {
        let old = self.profiles.get(&input.id).ok_or("missing")?;
        if old.revision != input.revision {
            return Err("revision conflict".into());
        }
        if self.fail_sync {
            return Err("model sync failed".into());
        }
        let mut next = old.clone();
        next.parameters_json = input.parameters_json.clone();
        next.revision += 1;
        self.profiles.insert(next.id.clone(), next);
        Ok(())
    }
    fn reset_model_parameters(
        &mut self,
        ids: &[String],
        parameters: &str,
        at: i64,
    ) -> Result<(), String> {
        if self.fail_sync {
            return Err("model sync failed".into());
        }
        for id in ids {
            let row = self.profiles.get_mut(id).ok_or("missing")?;
            row.parameters_json = parameters.into();
            row.updated_at_ms = at;
        }
        Ok(())
    }
    fn replace_model_profiles(
        &mut self,
        provider: &str,
        runtime: &str,
        profiles: &[StoredModelProfile],
        _: i64,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> Result<(), String> {
        if self.fail_sync {
            return Err("model sync failed".into());
        }
        let actual = self
            .profiles
            .values()
            .filter(|profile| {
                profile.provider == provider
                    && profile.runtime == runtime
                    && profile.deleted_at_ms.is_none()
            })
            .map(|profile| (profile.id.clone(), profile.revision))
            .collect::<std::collections::BTreeMap<_, _>>();
        if &actual != expected_revisions {
            return Err("revision_conflict: model catalog changed".into());
        }
        self.profiles
            .retain(|_, v| v.provider != provider || v.runtime != runtime);
        for row in profiles {
            self.profiles.insert(row.id.clone(), row.clone());
        }
        Ok(())
    }
    fn upsert_model_profile(&mut self, input: &StoredModelProfile) -> Result<(), String> {
        self.profiles.insert(input.id.clone(), input.clone());
        Ok(())
    }
    fn model_profile(&self, id: &str) -> Result<Option<StoredModelProfile>, String> {
        Ok(self.profiles.get(id).cloned())
    }
    fn list_model_profiles(&self) -> Result<Vec<StoredModelProfile>, String> {
        Ok(self.profiles.values().cloned().collect())
    }
    fn delete_model_profile(&mut self, id: &str, _: i64) -> Result<bool, String> {
        Ok(self.profiles.remove(id).is_some())
    }
    fn report_model_runtime(&mut self, input: &StoredModelRuntime) -> Result<(), String> {
        self.runtime = Some(input.clone());
        Ok(())
    }
    fn model_runtime(&self, host: &str) -> Result<Option<StoredModelRuntime>, String> {
        Ok(self.runtime.clone().filter(|r| r.host_id == host))
    }
}
fn profile() -> ModelProfile {
    ModelProfile {
        id: "model".into(),
        display_name: "Model".into(),
        provider: "provider".into(),
        model_name: "model".into(),
        runtime: "remote".into(),
        source_json: "{}".into(),
        parameters_json: "{}".into(),
        selected: true,
        enabled: true,
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}
#[test]
fn non_sqlite_model_commands_use_repository_and_preserve_failure() {
    let mut repository = Repository::default();
    execute_model_command(
        &mut repository,
        ModelCommand::UpsertProfile { input: profile() },
    )
    .unwrap();
    let mut changed = profile();
    changed.parameters_json = r#"{"temperature":0.2}"#.into();
    execute_model_command(
        &mut repository,
        ModelCommand::ConfigureParameters {
            input: changed.clone(),
        },
    )
    .unwrap();
    let result = execute_model_command(
        &mut repository,
        ModelCommand::GetProfile { id: "model".into() },
    )
    .unwrap();
    assert_eq!(result["parameters_json"], changed.parameters_json);
    assert!(execute_model_command(
        &mut repository,
        ModelCommand::ConfigureParameters {
            input: changed.clone()
        }
    )
    .unwrap_err()
    .contains("revision conflict"));
    changed.revision = 2;
    repository.fail_sync = true;
    assert!(execute_model_command(
        &mut repository,
        ModelCommand::ConfigureParameters { input: changed }
    )
    .unwrap_err()
    .contains("sync failed"));
    assert_eq!(
        execute_model_command(
            &mut repository,
            ModelCommand::GetProfile { id: "model".into() }
        )
        .unwrap(),
        result
    );
}
#[test]
fn non_sqlite_runtime_reports_are_typed_and_reject_unknown_phase() {
    let mut repository = Repository::default();
    let runtime = ModelRuntime {
        host_id: "host".into(),
        profile_id: "model".into(),
        phase: ModelRuntimePhase::Ready,
        endpoint_json: Some("{}".into()),
        process_id: None,
        error: None,
        started_at_ms: Some(1),
        stopped_at_ms: None,
        updated_at_ms: 1,
    };
    execute_model_command(
        &mut repository,
        ModelCommand::ReportRuntime { input: runtime },
    )
    .unwrap();
    assert_eq!(
        execute_model_command(
            &mut repository,
            ModelCommand::GetRuntime {
                host_id: "host".into()
            }
        )
        .unwrap()["phase"],
        "ready"
    );
    repository.runtime.as_mut().unwrap().phase = "untrusted phase".into();
    assert!(execute_model_command(
        &mut repository,
        ModelCommand::GetRuntime {
            host_id: "host".into()
        }
    )
    .is_err());
}
