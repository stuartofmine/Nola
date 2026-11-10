use nola_application::{ApplicationCommand, SopRunCommand, SopRunProjection};
use nola_core::agent::sop::{SopDefinition, SopRunEvent};
use serde_json::Value;

use super::NolaStore;

impl NolaStore {
    pub fn visible_active_sop_runs(
        &self,
        query: nola_store_port::sop_run::VisibleActiveSopRuns,
    ) -> Result<Vec<SopRunProjection>, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::VisibleActive { query },
        })
    }

    pub fn create_sop_run(
        &self,
        definition: SopDefinition,
        run_id: String,
        inputs: Value,
        owner_session_id: Option<String>,
        project_root: String,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::Create {
                definition,
                run_id,
                inputs,
                owner_session_id,
                project_root,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn apply_sop_run_event(
        &self,
        run_id: String,
        expected_revision: u64,
        event: SopRunEvent,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::Apply {
                run_id,
                expected_revision,
                event,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn sop_run(&self, run_id: String) -> Result<Option<SopRunProjection>, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::Load { run_id },
        })
    }

    pub fn validate_sop_invocation(
        &self,
        run_id: String,
        step_id: String,
        project_root: String,
        owner_session_id: String,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::ValidateInvocation {
                run_id,
                step_id,
                project_root,
                owner_session_id,
            },
        })
    }

    pub fn validate_sop_permission_invocation(
        &self,
        run_id: String,
        step_id: String,
        project_root: String,
        owner_session_id: String,
        reference_id: String,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::ValidatePermissionInvocation {
                run_id,
                step_id,
                project_root,
                owner_session_id,
                reference_id,
            },
        })
    }

    pub fn ensure_sop_permission_wait(
        &self,
        run_id: String,
        step_id: String,
        reference_id: String,
        reason: String,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::EnsurePermissionWait {
                run_id,
                step_id,
                reference_id,
                reason,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn ensure_sop_wait(
        &self,
        run_id: String,
        step_id: String,
        wait: nola_core::agent::SopWaitRecord,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::EnsureWait {
                run_id,
                step_id,
                wait,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn resolve_sop_permission_wait(
        &self,
        run_id: String,
        step_id: String,
        reference_id: String,
        disposition: nola_core::agent::SopWaitDisposition,
        evidence_refs: Vec<String>,
    ) -> Result<SopRunProjection, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::ResolvePermissionWait {
                run_id,
                step_id,
                reference_id,
                disposition,
                evidence_refs,
                now_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn list_sop_runs(
        &self,
        project_root: Option<String>,
        owner_session_id: Option<String>,
        limit: usize,
    ) -> Result<Vec<SopRunProjection>, String> {
        self.application()?.execute(ApplicationCommand::SopRun {
            command: SopRunCommand::List {
                project_root,
                owner_session_id,
                limit,
            },
        })
    }
}
