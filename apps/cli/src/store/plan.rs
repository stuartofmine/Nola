use nola_application::{
    ApplicationCommand, ConversationPlan, ConversationPlanCommand, ConversationPlanDiff,
    ConversationPlanRevision, ConversationPlanSopIdentity, ConversationPlanStepEdit,
};
use serde_json::Value;

use super::NolaStore;

impl NolaStore {
    pub fn save_session_plan(
        &self,
        session_id: &str,
        expected_revision: Option<i64>,
        sop: &ConversationPlanSopIdentity,
        todos: &Value,
    ) -> Result<(), String> {
        let mut application = crate::application_port::CliApplicationPort::persistent(&self.path)?;
        let _: ConversationPlan = application.execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                conversation_id: session_id.to_string(),
                expected_revision,
                sop: sop.clone(),
                todos: todos.clone(),
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })?;
        Ok(())
    }

    pub fn session_plan(&self, session_id: &str) -> Result<Option<Value>, String> {
        let plan = self.session_plan_record(session_id)?;
        plan.map(|plan| serde_json::to_value(plan.todos).map_err(|error| error.to_string()))
            .transpose()
    }

    pub fn session_plan_record(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationPlan>, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: session_id.to_string(),
                },
            },
        )
    }

    pub fn set_session_plan_approval(
        &self,
        session_id: &str,
        expected_revision: i64,
        step_ids: Vec<String>,
        approved: bool,
    ) -> Result<ConversationPlan, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::SetApproval {
                    conversation_id: session_id.to_string(),
                    expected_revision,
                    step_ids,
                    decision: if approved {
                        nola_application::ConversationPlanApprovalStatus::Approved
                    } else {
                        nola_application::ConversationPlanApprovalStatus::Rejected
                    },
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        )
    }

    pub fn edit_session_plan_step(
        &self,
        session_id: &str,
        expected_revision: i64,
        step_id: &str,
        edit: ConversationPlanStepEdit,
    ) -> Result<ConversationPlan, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::EditStep {
                    conversation_id: session_id.to_string(),
                    expected_revision,
                    step_id: step_id.to_string(),
                    edit,
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        )
    }

    pub fn freeze_session_plan(
        &self,
        session_id: &str,
        expected_revision: i64,
    ) -> Result<ConversationPlan, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Freeze {
                    conversation_id: session_id.to_string(),
                    expected_revision,
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        )
    }

    pub fn replan_session_plan(
        &self,
        session_id: &str,
        expected_revision: i64,
        sop: &ConversationPlanSopIdentity,
        todos: &Value,
        reason: &str,
    ) -> Result<ConversationPlan, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Replan {
                    conversation_id: session_id.to_string(),
                    expected_revision,
                    sop: sop.clone(),
                    todos: todos.clone(),
                    reason: reason.to_string(),
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        )
    }

    pub fn session_plan_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<ConversationPlanRevision>, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::History {
                    conversation_id: session_id.to_string(),
                },
            },
        )
    }

    pub fn session_plan_diff(
        &self,
        session_id: &str,
        from_revision: i64,
        to_revision: i64,
    ) -> Result<ConversationPlanDiff, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Diff {
                    conversation_id: session_id.to_string(),
                    from_revision,
                    to_revision,
                },
            },
        )
    }
}
