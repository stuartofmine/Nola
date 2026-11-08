// SPDX-License-Identifier: Apache-2.0
//! User questions are durable data requests, never permission grants.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
    pub allow_freeform: bool,
    pub required: bool,
    pub max_text_chars: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionSet { pub questions: Vec<UserQuestion> }

fn bounded(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= limit
}
impl QuestionSet {
    pub fn validate(&self) -> Result<(), String> {
        if self.questions.is_empty() || self.questions.len() > 16 { return Err("question set must contain 1..16 questions".into()); }
        let mut ids = BTreeSet::new();
        for q in &self.questions {
            if !bounded(&q.id, 128) || !ids.insert(&q.id) || !bounded(&q.prompt, 8192)
                || q.options.len() > 32 || (!q.allow_freeform && q.options.is_empty())
                || q.max_text_chars == 0 || q.max_text_chars > 65536 {
                return Err("question identity, prompt or response bounds are invalid".into());
            }
            let mut options = BTreeSet::new();
            for option in &q.options {
                if !bounded(&option.id,128) || !options.insert(&option.id) || !bounded(&option.label,1024)
                    || option.description.as_ref().is_some_and(|text| text.chars().count()>4096) {
                    return Err("question option identity or content is invalid".into());
                }
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self).map_err(|e|e.to_string())?)))
    }
    pub fn validate_response(&self, response: &UserInputWaitResponse) -> Result<(), String> {
        self.validate()?;
        match response {
            UserInputWaitResponse::Decline { reason } => {
                if reason.as_ref().is_some_and(|text| text.chars().count()>8192) { return Err("decline reason exceeds bound".into()); }
            }
            UserInputWaitResponse::Answers { answers } => {
                let mut seen = BTreeSet::new();
                for answer in answers {
                    let q = self.questions.iter().find(|q| q.id == answer.question_id).ok_or("unknown question identity")?;
                    if !seen.insert(&answer.question_id) { return Err("duplicate question answer".into()); }
                    match &answer.value {
                        QuestionAnswerValue::Option { option_id } if q.options.iter().any(|option| &option.id==option_id) => {}
                        QuestionAnswerValue::Text { text } if q.allow_freeform && bounded(text,q.max_text_chars as usize) => {}
                        _ => return Err("answer does not match the published question schema".into()),
                    }
                }
                if self.questions.iter().any(|q| q.required && !seen.contains(&q.id)) {
                    return Err("required question has no answer".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
pub enum QuestionAnswerValue { Option { option_id: String }, Text { text: String } }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionAnswer { pub question_id: String, pub value: QuestionAnswerValue }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
pub enum UserInputWaitResponse { Answers { answers: Vec<QuestionAnswer> }, Decline { reason: Option<String> } }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondToUserInputWait {
    pub command_id: String,
    pub wait_id: String,
    pub expected_revision: u64,
    pub response_schema_digest: String,
    pub response: UserInputWaitResponse,
}
impl RespondToUserInputWait {
    pub fn digest(&self) -> Result<String,String> {
        if !bounded(&self.command_id,256) || !bounded(&self.wait_id,256) || self.expected_revision==0 {
            return Err("wait response identity or revision is invalid".into());
        }
        let mut canonical = self.clone();
        if let UserInputWaitResponse::Answers { answers } = &mut canonical.response {
            answers.sort_by(|a,b| a.question_id.cmp(&b.question_id));
        }
        Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&canonical).map_err(|e|e.to_string())?)))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
pub enum UserInputWaitState { Open, Resolved { command_id:String,response_digest:String }, Withdrawn { reason:String } }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserInputWait {
    pub wait_id: String,
    pub core_session_id: String,
    pub turn_id: String,
    pub originating_call_id: Option<String>,
    pub revision: u64,
    pub created_at_revision: u64,
    pub question_set: QuestionSet,
    pub response_schema_digest: String,
    pub state: UserInputWaitState,
}
impl UserInputWait {
    pub fn is_open(&self) -> bool { matches!(self.state,UserInputWaitState::Open) }
    pub fn validate(&self) -> Result<(),String> {
        if !bounded(&self.wait_id,256) || !bounded(&self.core_session_id,256) || !bounded(&self.turn_id,256)
            || self.originating_call_id.as_ref().is_some_and(|id| !bounded(id,256))
            || self.created_at_revision==0 || self.revision==0 || self.revision>i64::MAX as u64
            || self.question_set.digest()?!=self.response_schema_digest {
            return Err("wait identity, revision or question schema is invalid".into());
        }
        match &self.state {
            UserInputWaitState::Open if self.revision!=1 => return Err("open wait revision must be one".into()),
            UserInputWaitState::Resolved {command_id,response_digest} if self.revision!=2 || !bounded(command_id,256)
                || response_digest.len()!=64 || !response_digest.bytes().all(|b|b.is_ascii_hexdigit()) => return Err("invalid resolved wait receipt".into()),
            UserInputWaitState::Withdrawn {reason} if self.revision!=2 || !bounded(reason,8192) => return Err("invalid wait withdrawal".into()),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn questions() -> QuestionSet {
        QuestionSet { questions: vec![UserQuestion { id:"scope".into(),prompt:"Which scope?".into(),
            options:vec![QuestionOption {id:"one".into(),label:"One".into(),description:None}],
            allow_freeform:false,required:true,max_text_chars:32 }] }
    }
    #[test]
    fn schema_rejects_missing_duplicate_and_unknown_stable_id() {
        let schema=questions();schema.validate().unwrap();
        for answers in [vec![], vec![QuestionAnswer {question_id:"scope".into(), value:QuestionAnswerValue::Option {option_id:"One".into()}}],
            vec![QuestionAnswer {question_id:"scope".into(),value:QuestionAnswerValue::Text {text:"one".into()}}]] {
            assert!(schema.validate_response(&UserInputWaitResponse::Answers {answers}).is_err());
        }
        let answer=QuestionAnswer {question_id:"scope".into(),value:QuestionAnswerValue::Option {option_id:"one".into()}};
        assert!(schema.validate_response(&UserInputWaitResponse::Answers {answers:vec![answer.clone(),answer]}).is_err());
        let mut duplicate=schema.clone();duplicate.questions.push(schema.questions[0].clone());assert!(duplicate.validate().is_err());
        assert!(schema.validate_response(&UserInputWaitResponse::Decline {reason:None}).is_ok());
    }
    #[test]
    fn response_digest_is_order_independent_but_content_and_revision_bound() {
        let mut input=RespondToUserInputWait {command_id:"response".into(),wait_id:"wait".into(),expected_revision:1,
            response_schema_digest:questions().digest().unwrap(),response:UserInputWaitResponse::Answers {answers:vec![
                QuestionAnswer {question_id:"b".into(),value:QuestionAnswerValue::Option {option_id:"one".into()}},
                QuestionAnswer {question_id:"a".into(),value:QuestionAnswerValue::Text {text:"data".into()}}]}};
        let digest=input.digest().unwrap();
        if let UserInputWaitResponse::Answers {answers}=&mut input.response {answers.reverse();}
        assert_eq!(input.digest().unwrap(),digest);
        input.expected_revision=2;assert_ne!(input.digest().unwrap(),digest);
    }
}
