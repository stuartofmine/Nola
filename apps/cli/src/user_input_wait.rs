// SPDX-License-Identifier: Apache-2.0
//! Presentation of committed questions. Closing a form never resolves a wait.
use nola_domain::user_input_wait::{QuestionAnswer, QuestionAnswerValue, RespondToUserInputWait, UserInputWaitResponse};
use nola_store_port::user_input_wait::StoredUserInputWait;
use crate::application_port::CliApplicationPort;

pub(crate) fn open(conversation: &str) -> Result<Vec<StoredUserInputWait>, String> {
    CliApplicationPort::persistent_default()?.execute(nola_application::ApplicationCommand::ListOpenUserInputWaits {
        conversation_id: conversation.to_owned(), run_id: None,
    })
}

/// Emits only records loaded from the durable Application query. Failed output
/// is not an answer and does not alter the wait; clients can query it on reopen.
pub(crate) fn publish_open(conversation: &str) -> Result<(), String> {
    for record in open(conversation)? {
        crate::native_host_output::emit(serde_json::json!({"type":"user_input_wait_changed","record":record}))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn publish(conversation: &str, wait_id: &str) -> Result<(), String> {
    let record: Option<StoredUserInputWait> = CliApplicationPort::persistent_default()?.execute(
        nola_application::ApplicationCommand::LoadUserInputWait { conversation_id: conversation.to_owned(), wait_id: wait_id.to_owned() })?;
    if let Some(record) = record {
        crate::native_host_output::emit(serde_json::json!({"type":"user_input_wait_changed","record":record}))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn owned(conversation: &str, task_id: Option<String>) -> Result<Vec<nola_store_port::user_input_wait::OwnedTaskUserInputWait>, String> {
    CliApplicationPort::persistent_default()?.execute(nola_application::ApplicationCommand::ListOwnedTaskUserInputWaits {
        owner_conversation_id: conversation.to_owned(), task_id,
    })
}

fn collect(record: &StoredUserInputWait, task_id: Option<&str>, interaction: &dyn crate::runtime_interaction::RuntimeInteraction) -> Result<Option<UserInputWaitResponse>, String> {
    use crate::runtime_interaction::{UserQuestionAnswer, UserQuestionPrompt};
    let mut answers = Vec::new();
    for question in &record.wait.question_set.questions {
        let prompt = UserQuestionPrompt {
            header: match task_id {
                Some(id) => format!("{} · Task {id}", crate::i18n::interaction_text().decision_required),
                None => crate::i18n::interaction_text().decision_required.into(),
            }, question: question.prompt.clone(),
            options: question.options.iter().map(|option| (option.label.clone(), option.description.clone().unwrap_or_default())).collect(),
            allow_freeform: question.allow_freeform,
        };
        let value = match interaction.request_user_question(&prompt) {
            UserQuestionAnswer::Selected { index, .. } => QuestionAnswerValue::Option {
                option_id: question.options.get(index).ok_or("selected option no longer exists")?.id.clone(),
            },
            UserQuestionAnswer::Freeform(text) => QuestionAnswerValue::Text { text },
            UserQuestionAnswer::Declined => return Ok(Some(UserInputWaitResponse::Decline { reason: None })),
            UserQuestionAnswer::Cancelled | UserQuestionAnswer::Unavailable => return Ok(None),
        };
        answers.push(QuestionAnswer { question_id: question.id.clone(), value });
    }
    let response = UserInputWaitResponse::Answers { answers };
    record.wait.question_set.validate_response(&response)?;
    Ok(Some(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_interaction_preserves_committed_wait_without_response() {
        let record=child_record();
        let result=present_records("parent",vec![(Some("task".into()),record)],
            |record,task|collect(record,task,&crate::runtime_interaction::UnavailableInteraction),
            |_|panic!("unavailable presentation cannot commit an answer")).unwrap();
        assert!(result.deferred);
        assert!(result.resume_run.is_none());
    }
    fn child_record() -> StoredUserInputWait {
        let question_set: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(serde_json::json!({
            "questions":[{"id":"scope","prompt":"Choose","options":[{"id":"local","label":"Local"}],
                "allow_freeform":true,"required":true,"max_text_chars":100}]
        })).unwrap();
        StoredUserInputWait { conversation_id:"child".into(), run_id:"child-run".into(),
            wait:nola_domain::user_input_wait::UserInputWait {
                wait_id:"wait".into(),core_session_id:"child-core".into(),turn_id:"turn".into(),
                originating_call_id:Some("call".into()),revision:1,created_at_revision:2,
                response_schema_digest:question_set.digest().unwrap(),question_set,
                state:nola_domain::user_input_wait::UserInputWaitState::Open,
            }}
    }

    #[test]
    fn owned_child_question_keeps_original_owner_and_never_resumes_in_parent() {
        let record=child_record();
        for policy in [nola_application::WaitResumePolicy::TaskScheduler,nola_application::WaitResumePolicy::ConversationRun] {
            let resumed=present_records("parent",vec![(Some("task".into()),record.clone())],
                |shown,task| { assert_eq!(task,Some("task"));assert_eq!(shown.run_id,"child-run");
                    Ok(Some(UserInputWaitResponse::Decline{reason:None})) },
                |command| { assert_eq!(command.conversation_id,"child");assert_eq!(command.run_id,"child-run");
                    assert_eq!(command.response.wait_id,"wait");assert_eq!(command.response.expected_revision,1);
                    assert_eq!(command.response.response_schema_digest,record.wait.response_schema_digest);
                    assert!(matches!(command.response.response,UserInputWaitResponse::Decline{..}));
                    Ok(nola_application::ConversationWaitResponse::Replay{wait:record.clone(),resume_policy:policy.clone()}) }
            ).unwrap();
            assert!(resumed.resume_run.is_none());
        }
    }

    #[test]
    fn closing_child_form_never_submits_or_consumes_later_questions() {
        let record=child_record(); let mut displayed=0;
        let resumed=present_records("parent",vec![(Some("task".into()),record.clone()),(Some("task2".into()),record)],
            |_,_| {displayed+=1;Ok(None)}, |_| panic!("closing is not a response")).unwrap();
        assert_eq!(displayed,1);assert!(resumed.resume_run.is_none());assert!(resumed.deferred);
    }

    #[test]
    fn wait_conflict_notice_distinguishes_cancellation_from_answered_or_revised() {
        let record=child_record();
        for (reason,expected) in [
            (nola_application::WaitConflictReason::CancellationRequested,"任务已请求取消"),
            (nola_application::WaitConflictReason::SchemaChanged,"问题内容或选项已更新"),
            (nola_application::WaitConflictReason::RevisionChanged,"问题版本已变化"),
            (nola_application::WaitConflictReason::WaitNotOpen,"问题已关闭"),
        ] {
            let result=present_records("parent",vec![(Some("task".into()),record.clone())],
                |_,_|Ok(Some(UserInputWaitResponse::Decline{reason:None})),
                |_|Ok(nola_application::ConversationWaitResponse::Conflict{wait:record.clone(),reason:reason.clone()})).unwrap();
            assert!(result.notice.unwrap().starts_with(expected));
            assert!(result.deferred);
            assert!(result.resume_run.is_none());
        }
    }

    #[test]
    fn child_question_conflict_is_not_a_successful_resume() {
        let record=child_record();
        assert!(present_records("parent",vec![(Some("task".into()),record.clone())],
            |_,_| Ok(Some(UserInputWaitResponse::Decline{reason:None})),
            |_| Ok(nola_application::ConversationWaitResponse::Conflict{wait:record.clone(),reason:nola_application::WaitConflictReason::WaitNotOpen})).unwrap().notice.is_some());
    }
    #[test]
    fn committed_parent_resume_survives_later_child_conflict_error_or_escape() {
        for failure in ["conflict", "transport", "collect", "escape"] {
            let child = child_record();
            let mut parent = child.clone(); parent.conversation_id = "parent".into(); parent.run_id = "parent-run".into();
            let outcome = present_records("parent", vec![(None,parent.clone()),(Some("task".into()),child.clone())],
                |_, task| {
                    if task.is_some() && failure == "escape" { return Ok(None); }
                    if task.is_some() && failure == "collect" { return Err("invalid answer".into()); }
                    Ok(Some(UserInputWaitResponse::Decline {reason:None}))
                }, |command| {
                    if command.conversation_id == "parent" {
                        return Ok(nola_application::ConversationWaitResponse::Replay {wait:parent.clone(),resume_policy:nola_application::WaitResumePolicy::ConversationRun});
                    }
                    if failure == "transport" { return Err("transport unavailable".into()); }
                    Ok(nola_application::ConversationWaitResponse::Conflict {wait:child.clone(),reason:nola_application::WaitConflictReason::WaitNotOpen})
                }).unwrap();
            assert_eq!(outcome.resume_run.as_deref(),Some("parent-run"),"{failure}");
            assert!(outcome.deferred,"{failure}");
            assert_eq!(outcome.notice.is_some(),failure != "escape");
        }
    }

    #[test]
    fn another_open_parent_question_suppresses_resume_after_escape() {
        let mut record=child_record(); record.conversation_id="parent".into();
        let mut shown=0;
        let outcome=present_records("parent",vec![(None,record.clone()),(None,record.clone())],
            |_,_| { shown+=1; if shown==2 {Ok(None)} else {Ok(Some(UserInputWaitResponse::Decline{reason:None}))} },
            |_| Ok(nola_application::ConversationWaitResponse::Replay{wait:record.clone(),resume_policy:nola_application::WaitResumePolicy::ConversationRun})).unwrap();
        assert!(outcome.resume_run.is_none());assert!(outcome.deferred);assert!(outcome.notice.is_none());
    }

    #[test]
    fn explicit_option_identity_is_not_a_display_label() {
        let questions: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(serde_json::json!({
            "questions":[{"id":"scope","prompt":"Which scope?","options":[
                {"id":"local","label":"Same label"},{"id":"global","label":"Same label"}],
                "allow_freeform":false,"required":true,"max_text_chars":100}]
        })).unwrap();
        assert!(questions.validate_response(&UserInputWaitResponse::Answers { answers: vec![QuestionAnswer {
            question_id: "scope".into(), value: QuestionAnswerValue::Option { option_id: "global".into() }
        }] }).is_ok());
        assert!(questions.validate_response(&UserInputWaitResponse::Answers { answers: vec![QuestionAnswer {
            question_id: "scope".into(), value: QuestionAnswerValue::Option { option_id: "Same label".into() }
        }] }).is_err());
        assert!(questions.validate_response(&UserInputWaitResponse::Decline { reason: None }).is_ok());
    }
}

/// Invoked outside the running worker/composer input lock. Every form answers
/// its captured conversation/Run, never whatever session is currently selected.
pub(crate) struct PresentationOutcome {
    pub resume_run: Option<String>,
    pub notice: Option<String>,
    pub deferred: bool,
}

pub(crate) fn present_pending(conversation: &str, include_children: bool, interaction: &dyn crate::runtime_interaction::RuntimeInteraction) -> Result<PresentationOutcome, String> {
    let mut records: Vec<_> = open(conversation)?.into_iter().map(|record| (None, record)).collect();
    if include_children {
        records.extend(owned(conversation, None)?.into_iter().map(|record| (Some(record.task_id), record.stored)));
    }
    let mut outcome = present_records(conversation, records, |record, task| collect(record, task, interaction), |command| {
        CliApplicationPort::persistent_default()?.execute(
            nola_application::ApplicationCommand::RespondToConversationWait { input: command })
    })?;
    if outcome.resume_run.is_some() {
        match open(conversation) {
            Ok(waits) if !waits.is_empty() => { outcome.resume_run = None; outcome.deferred = true; }
            Err(error) => { outcome.resume_run = None; outcome.notice = Some(error); outcome.deferred = true; }
            _ => {}
        }
    }
    Ok(outcome)
}

fn present_records(
    conversation: &str,
    records: Vec<(Option<String>, StoredUserInputWait)>,
    mut collect: impl FnMut(&StoredUserInputWait, Option<&str>) -> Result<Option<UserInputWaitResponse>, String>,
    mut respond: impl FnMut(nola_application::RespondToConversationWait) -> Result<nola_application::ConversationWaitResponse, String>,
) -> Result<PresentationOutcome, String> {
    let mut outcome = PresentationOutcome { resume_run: None, notice: None, deferred: false };
    let mut remaining_own = records.iter().filter(|(task, record)| task.is_none() && record.conversation_id == conversation).count();
    for (task_id, record) in records {
        let response = match collect(&record, task_id.as_deref()) {
            Ok(Some(response)) => response,
            Ok(None) => { outcome.deferred = true; break; }
            Err(error) => { outcome.notice = Some(error); outcome.deferred = true; break; }
        };
        let committed_at_ms = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_millis() as i64,
            Err(error) => { outcome.notice = Some(error.to_string()); outcome.deferred = true; break; }
        };
        let command = nola_application::RespondToConversationWait {
            conversation_id: record.conversation_id.clone(), run_id: record.run_id.clone(),
            response: RespondToUserInputWait {
                command_id: uuid::Uuid::new_v4().to_string(), wait_id: record.wait.wait_id.clone(),
                expected_revision: record.wait.revision,
                response_schema_digest: record.wait.response_schema_digest.clone(), response,
            },
            committed_at_ms,
        };
        let result = match respond(command) {
            Ok(result) => result,
            Err(error) => { outcome.notice = Some(error); outcome.deferred = true; break; }
        };
        match result {
            nola_application::ConversationWaitResponse::Applied { resume_policy, .. } |
            nola_application::ConversationWaitResponse::Replay { resume_policy, .. } => {
                if task_id.is_none() && record.conversation_id == conversation && matches!(resume_policy, nola_application::WaitResumePolicy::ConversationRun) {
                    outcome.resume_run = Some(record.run_id);
                }
                if task_id.is_none() && record.conversation_id == conversation {
                    remaining_own = remaining_own.saturating_sub(1);
                }
            }
            nola_application::ConversationWaitResponse::Conflict { reason, .. } => {
                outcome.notice = Some(match reason {
                    nola_application::WaitConflictReason::CancellationRequested => "任务已请求取消，本次回答未提交；等待取消完成。",
                    nola_application::WaitConflictReason::SchemaChanged => "问题内容或选项已更新，请重新打开待回答问题。",
                    nola_application::WaitConflictReason::RevisionChanged => "问题版本已变化，本次回答未提交；请重新打开待回答问题。",
                    nola_application::WaitConflictReason::WaitNotOpen => "问题已关闭（已回答或撤回），本次回答未提交。",
                }.into());
                outcome.deferred = true;
                break;
            }
        }
    }
    if remaining_own > 0 { outcome.resume_run = None; }
    Ok(outcome)
}
