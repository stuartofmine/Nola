// SPDX-License-Identifier: Apache-2.0
//! Runtime output contracts. Claims below are model declarations read from
//! committed evidence, never acceptance or authority to complete a Goal.
use nola_core::agent::capability::SkillRuntimeEvent;
use nola_core::agent::protocol::AgentEvent;
use nola_core::agent::task::TaskHandle;
use nola_core::agent_trace::AgentTraceEvent;
use nola_domain::core_event::CoreEventEnvelope;

/// Transport-only sink. Canonical state and completion remain in Application.
pub trait RuntimeOutputPort {
    fn bind_execution_control(&mut self, context: crate::execution_context::ExecutionContext);
    fn submission_admission(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        assistant_message_id: &str,
        rejection: Option<&str>,
    );
    fn defer_terminal_transport(&mut self);
    fn begin_recovery_transport_isolation(&mut self);
    fn end_recovery_transport_isolation(&mut self);
    fn flush_deferred_terminal_transport(&mut self);
    fn discard_deferred_terminal_transport(&mut self);
    fn set_session_id(&mut self, session_id: Option<String>);
    fn set_host_turn_identity(
        &mut self,
        run_id: Option<String>,
        assistant_message_id: Option<String>,
    );
    fn set_next_trace_message_id(&mut self, message_id: Option<String>);
    fn begin_core_trace(&mut self, trace: &AgentTraceEvent);
    fn discard_core_state_snapshot_trace(&mut self);
    fn core_turn_terminal(&mut self, envelope: &CoreEventEnvelope<AgentEvent>);
    fn core_task_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        task_projection_revision: u64,
        task: &TaskHandle,
    );
    fn core_skill_runtime_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        skill_runtime_revision: u64,
        update: &SkillRuntimeEvent,
    );
    fn core_capabilities_activated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        source: &str,
        capability_ids: &[String],
    );
    fn core_permission_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        revision: u64,
        pending: &[nola_core::agent::permissions::PendingPermissionRequest],
        active_policy: Option<&nola_domain::ActivePermissionPolicy>,
    );
    fn committed_assistant_journal(
        &mut self,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
    );
    fn committed_assistant_run_terminal(
        &mut self,
        seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    );
    fn core_memory_runtime_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        projection: &nola_core::agent::MemoryRuntimeProjection,
    );
    fn end_core_event(&mut self);
    fn end_core_event_without_output(&mut self);
    fn handoff_core_trace_to_host_event(&mut self);
    fn set_context_usage_without_breakdown(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        message_count: Option<usize>,
        tool_count: usize,
        cache_usage: Option<nola_domain::PromptCacheUsage>,
    );
    fn is_json_transport(&self) -> bool;
    fn status(&mut self, phase: &str, message: &str);
    fn done(&mut self, reason: &str);
    fn error(&mut self, message: &str);
    fn journal_observer(
        &self,
    ) -> Option<Box<dyn crate::runtime_output::JournalPublicationObserver>>;
}

pub trait JournalPublicationObserver: std::fmt::Debug + Send {
    fn observe(
        &mut self,
        port: &mut crate::application_port::CliApplicationPort,
        cursor: &nola_domain::assistant_journal::AssistantJournalCursor,
    ) -> Result<(), String>;
}

pub use nola_runtime_driver::goal::GoalCompleteSignal;

pub(crate) fn goal_claim_from_journal(
    journal: &crate::source_journal::SourceRunJournal,
) -> Result<Option<GoalCompleteSignal>, String> {
    use nola_domain::assistant_journal_operations::{JournalContentField, JournalPartMetadata};
    if journal.seal.is_none() {
        return Err("Goal claim requires the exact Run terminal seal".into());
    }
    // Use sealed canonical ordering, not renderer arrival or map key order.
    let part = journal
        .ordered_parts()
        .into_iter()
        .filter(|part| part.head.identity.run_id == journal.run_id)
        .filter(|part| {
            matches!(&part.head.metadata,JournalPartMetadata::Tool(tool)
            if tool.capability_id=="goal_complete")
        })
        .last();
    let Some(part) = part else { return Ok(None) };
    if !matches!(&part.head.metadata, JournalPartMetadata::Tool(tool)
        if tool.state == nola_domain::ActivityState::Completed)
    {
        return Ok(None);
    }
    let output = part
        .structured(JournalContentField::ToolOutput)?
        .ok_or("Goal declaration has no committed structured output")?;
    GoalCompleteSignal::from_tool_output(&output).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn committed_claim_requires_seal_and_not_renderer() {
        let (_, mut journal) = crate::source_journal::source_tests::completed_source(
            &[(
                "goal_complete",
                json!({"goal_status":"needs_user", "summary":"Need input", "claims":[]}),
            )],
            "No acceptance is inferred from this answer",
        );
        let claim = goal_claim_from_journal(&journal).unwrap().unwrap();
        assert_eq!(claim.status, "needs_user");
        assert!(claim.claims.is_empty());
        journal.seal = None;
        assert!(goal_claim_from_journal(&journal).is_err());
    }

    #[test]
    fn text_and_other_tools_do_not_manufacture_goal_claim() {
        let (_, journal) = crate::source_journal::source_tests::completed_source(
            &[(
                "read_file",
                json!({"goal_status":"completed", "summary":"Untrusted content"}),
            )],
            "goal_complete: completed",
        );
        assert_eq!(goal_claim_from_journal(&journal).unwrap(), None);
    }

    #[test]
    fn malformed_committed_declaration_is_not_silently_accepted() {
        let (_, journal) = crate::source_journal::source_tests::completed_source(
            &[("goal_complete", json!({"goal_status":"completed"}))],
            "done",
        );
        assert!(goal_claim_from_journal(&journal).is_err());
    }

    #[test]
    fn latest_failed_declaration_does_not_restore_an_earlier_claim() {
        use nola_domain::assistant_journal_operations::JournalPartMetadata;
        let value = json!({"goal_status":"needs_user","summary":"Ask owner"});
        let (_, mut journal) = crate::source_journal::source_tests::completed_source(
            &[("goal_complete", value.clone()), ("goal_complete", value)],
            "done",
        );
        let last = journal.ordered_parts().into_iter().filter(|part|
            matches!(&part.head.metadata, JournalPartMetadata::Tool(tool) if tool.capability_id == "goal_complete"))
            .last().unwrap().head.part_id.clone();
        let part = journal
            .parts
            .values_mut()
            .find(|part| part.head.part_id == last)
            .unwrap();
        if let JournalPartMetadata::Tool(tool) = &mut part.head.metadata {
            tool.state = nola_domain::ActivityState::Failed;
        }
        assert_eq!(goal_claim_from_journal(&journal).unwrap(), None);
    }

    #[test]
    fn unknown_status_and_empty_summary_are_invalid_claims() {
        for value in [
            json!({"goal_status":"unknown","summary":"x"}),
            json!({"goal_status":"completed","summary":"  "}),
        ] {
            let (_, journal) = crate::source_journal::source_tests::completed_source(
                &[("goal_complete", value)],
                "done",
            );
            assert!(goal_claim_from_journal(&journal).is_err());
        }
    }
}
