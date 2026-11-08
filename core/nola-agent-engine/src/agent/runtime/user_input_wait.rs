// SPDX-License-Identifier: Apache-2.0
//! Durable question rendezvous. Answers carry data, never authorization.
use super::*;
use crate::agent::host::HostToolOutcome;
use nola_domain::user_input_wait::{RespondToUserInputWait, UserInputWaitState};

impl AgentRuntime {
    /// Permission resolution has its own execution-admission path. Intercept
    /// only already-authorized question effects before any Host dispatch.
    pub(super) fn capture_authorized_question_effects(&mut self, events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>) -> Result<(), String> {
        let mut changed_turn = None;
        let pending = std::mem::take(effects);
        for effect in pending {
            if let AgentEffect::ExecuteTool { turn_id, call_id, tool_name, arguments, .. } = &effect {
                if tool_name == "request_user_input" {
                    let question_set: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())?;
                    let wait = nola_domain::user_input_wait::UserInputWait {
                        wait_id: self.session.allocate_id("user-input-wait"),
                        core_session_id: self.session.session_id.as_str().to_owned(), turn_id: turn_id.as_str().to_owned(),
                        originating_call_id: Some(call_id.as_str().to_owned()), revision: 1,
                        created_at_revision: self.session.journal.revision.checked_add(1).ok_or("Core revision exhausted")?,
                        response_schema_digest: question_set.digest()?, question_set, state: UserInputWaitState::Open,
                    };
                    let tool = self.session.execution.active_turn.as_mut().and_then(|turn| turn.tools.iter_mut().find(|tool| &tool.call_id == call_id)).ok_or("authorized question lost its tool")?;
                    tool.status = ToolCallStatus::AwaitingUserInput;
                    self.session.execution.user_input_waits.insert(wait.wait_id.clone(), wait.clone());
                    events.push(RuntimeEvent::UserInputWaitChanged { wait });
                    changed_turn = Some(turn_id.clone());
                    continue;
                }
            }
            effects.push(effect);
        }
        if let Some(turn_id) = changed_turn { self.schedule_tools(&turn_id, events, effects)?; }
        Ok(())
    }

    pub(super) fn user_input_response_is_replay(&self, input: &RespondToUserInputWait) -> Result<bool, String> {
        let digest = input.digest()?;
        for wait in self.session.execution.user_input_waits.values() {
            if let UserInputWaitState::Resolved { command_id, response_digest } = &wait.state {
                if command_id == &input.command_id {
                    return if wait.wait_id == input.wait_id && response_digest == &digest { Ok(true) }
                    else { Err("question response command identity was reused with different content".into()) };
                }
            }
        }
        Ok(false)
    }

    pub(super) fn respond_to_user_input_wait(&mut self, input: RespondToUserInputWait,
        events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>) -> Result<(), String> {
        let digest = input.digest()?;
        let wait = self.session.execution.user_input_waits.get(&input.wait_id).ok_or("unknown user input wait")?;
        if !wait.is_open() || wait.revision != input.expected_revision || wait.response_schema_digest != input.response_schema_digest {
            return Err("question response does not match the current open wait revision/schema".into());
        }
        wait.question_set.validate_response(&input.response)?;
        let turn = self.session.execution.active_turn.as_ref().ok_or("question has no owning Turn")?;
        if turn.turn_id.as_str() != wait.turn_id || turn.status.is_terminal() || turn.status == AgentTurnStatus::Cancelling {
            return Err("question response belongs to an inactive Turn".into());
        }
        let turn_id = turn.turn_id.clone();
        let call_id = wait.originating_call_id.clone();
        let wait = self.session.execution.user_input_waits.get_mut(&input.wait_id).expect("validated wait");
        wait.revision = wait.revision.checked_add(1).ok_or("wait revision exhausted")?;
        wait.state = UserInputWaitState::Resolved { command_id: input.command_id, response_digest: digest };
        events.push(RuntimeEvent::UserInputWaitChanged { wait: wait.clone() });
        if let Some(call_id) = call_id {
            let call_id = CallId::from_string(call_id);
            let tool = self.session.execution.active_turn.as_mut().and_then(|turn| turn.tools.iter_mut().find(|tool| tool.call_id == call_id))
                .ok_or("question has no owning tool call")?;
            if tool.status != ToolCallStatus::AwaitingUserInput { return Err("question tool is not waiting".into()); }
            // Use the canonical tool-result reducer; no Host execution is fabricated.
            tool.status = ToolCallStatus::Running;
            self.complete_tool(&turn_id, &call_id, None, serde_json::json!({"response": input.response}),
                HostToolOutcome::Succeeded, None, None, Vec::new(), None, false, events, effects)?;
        } else {
            let item = AgentItem::Message {
                id: ItemId::from_string(self.session.allocate_id("question-response")),
                role: AgentRole::User,
                content: serde_json::json!({"answered_wait_id": input.wait_id, "response": input.response}).to_string(),
            };
            self.session.record_item(item.clone());
            events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item });
            let turn = self.session.execution.active_turn.as_mut().expect("validated turn");
            turn.completion_outcome = None;
            turn.final_answer_draft = None;
            turn.final_answer_draft_part_id = None;
            turn.final_answer_audit_report = None;
            turn.final_answer_retry_requested = false;
            self.request_model(&turn_id, events, effects)?;
        }
        Ok(())
    }

    pub(super) fn open_review_user_input_wait(&mut self, turn_id: &TurnId,
        question_set: nola_domain::user_input_wait::QuestionSet, events: &mut Vec<RuntimeEvent>) -> Result<(), String> {
        let wait = nola_domain::user_input_wait::UserInputWait {
            wait_id: self.session.allocate_id("user-input-wait"),
            core_session_id: self.session.session_id.as_str().to_owned(), turn_id: turn_id.as_str().to_owned(),
            originating_call_id: None, revision: 1,
            created_at_revision: self.session.journal.revision.checked_add(1).ok_or("Core revision exhausted")?,
            response_schema_digest: question_set.digest()?, question_set,
            state: UserInputWaitState::Open,
        };
        self.session.execution.user_input_waits.insert(wait.wait_id.clone(), wait.clone());
        self.session.execution.active_turn.as_mut().ok_or("no active turn")?.status = AgentTurnStatus::AwaitingInput;
        events.push(RuntimeEvent::UserInputWaitChanged { wait });
        Ok(())
    }

    pub(super) fn withdraw_user_input_waits(&mut self, reason: &str, events: &mut Vec<RuntimeEvent>) {
        for wait in self.session.execution.user_input_waits.values_mut().filter(|wait| wait.is_open()) {
            wait.revision += 1;
            wait.state = UserInputWaitState::Withdrawn { reason: reason.to_owned() };
            events.push(RuntimeEvent::UserInputWaitChanged { wait: wait.clone() });
        }
    }

    pub(super) fn withdraw_call_input_waits(
        &mut self, turn_id: &TurnId, calls: &[CallId], reason: &str,
        events: &mut Vec<RuntimeEvent>,
    ) -> Result<(), String> {
        for wait in self.session.execution.user_input_waits.values_mut().filter(|wait|
            wait.is_open() && wait.turn_id == turn_id.as_str()
                && wait.originating_call_id.as_deref().is_some_and(|id|
                    calls.iter().any(|call| call.as_str() == id))) {
            wait.revision = wait.revision.checked_add(1).ok_or("user input wait revision exhausted")?;
            wait.state = UserInputWaitState::Withdrawn { reason: reason.to_owned() };
            events.push(RuntimeEvent::UserInputWaitChanged { wait: wait.clone() });
        }
        Ok(())
    }
}
