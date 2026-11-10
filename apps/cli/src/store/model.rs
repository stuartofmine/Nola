use super::NolaStore;
use crate::application_port::CliApplicationPort;
use nola_application::{ApplicationCommand, ModelCommand, ModelProfile, ModelRuntime};
use serde_json::Value;

impl NolaStore {
    pub fn replace_remote_model_profiles(&self, profiles: Vec<ModelProfile>, expected_revisions: std::collections::BTreeMap<String, i64>) -> Result<(), String> {
        let mut application = self.canonical_model_store()?;
        let _: Value = application.execute(ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "openai-compatible".into(), runtime: "remote".into(), profiles,
                updated_at_ms: chrono::Utc::now().timestamp_millis(), expected_revisions,
            },
        })?;
        Ok(())
    }

    pub fn configure_model_endpoint(&self, endpoint_id: &str, expected_revisions: std::collections::BTreeMap<String, i64>, name: String, base_url: String, wire_protocol: String, enabled: bool) -> Result<Vec<ModelProfile>, String> {
        self.canonical_model_store()?.execute(ApplicationCommand::Model {
            command: ModelCommand::ConfigureEndpoint {
                endpoint_id: endpoint_id.to_owned(), expected_revisions, name, base_url, wire_protocol, enabled,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }
    pub fn update_model_profile(&self, input: ModelProfile, expected_revision: u64) -> Result<(), String> {
        let mut application = self.canonical_model_store()?;
        let _: Value = application.execute_with_revision(expected_revision, ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input },
        })?;
        Ok(())
    }

    fn canonical_model_store(&self) -> Result<CliApplicationPort, String> {
        CliApplicationPort::persistent(&self.path)
    }

    pub fn upsert_model_profile(&self, input: ModelProfile) -> Result<(), String> {
        let mut application = self.canonical_model_store()?;
        let _: Value = application.execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input },
        })?;
        Ok(())
    }

    pub fn configure_model_parameters(
        &self,
        input: ModelProfile,
        expected_revision: u64,
    ) -> Result<(), String> {
        let mut application = self.canonical_model_store()?;
        let _: Value = application.execute_with_revision(
            expected_revision,
            ApplicationCommand::Model {
                command: ModelCommand::ConfigureParameters { input },
            },
        )?;
        Ok(())
    }

    pub fn model_profile(&self, id: &str) -> Result<Option<ModelProfile>, String> {
        self.canonical_model_store()?
            .execute(ApplicationCommand::Model {
                command: ModelCommand::GetProfile { id: id.to_string() },
            })
    }

    pub fn model_profiles(&self) -> Result<Vec<ModelProfile>, String> {
        self.canonical_model_store()?
            .execute(ApplicationCommand::Model {
                command: ModelCommand::ListProfiles,
            })
    }

    pub fn delete_model_profile(&self, id: &str, deleted_at_ms: i64) -> Result<bool, String> {
        self.canonical_model_store()?
            .execute(ApplicationCommand::Model {
                command: ModelCommand::DeleteProfile {
                    id: id.to_string(),
                    deleted_at_ms,
                },
            })
    }

    pub fn report_model_runtime(&self, input: ModelRuntime) -> Result<(), String> {
        let mut application = self.canonical_model_store()?;
        let _: Value = application.execute(ApplicationCommand::Model {
            command: ModelCommand::ReportRuntime { input },
        })?;
        Ok(())
    }
}
