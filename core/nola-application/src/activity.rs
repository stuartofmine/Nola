// SPDX-License-Identifier: Apache-2.0
//! Canonical execution history, separate from trace and private client summaries.
use serde::{Deserialize,Serialize};
use nola_store_port::{activity::*,consistent_read::ConsistentReadRepository,remote_session::RemoteSessionRepository};
use nola_domain::assistant_run_terminal::AssistantRunTerminalSeal;
pub use nola_store_port::activity::{ActivityCursor,ActivityQuery};
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ActivityEntry {
    pub run_id:String,pub conversation_id:String,pub assistant_message_id:String,
    pub source:String,pub title:String,pub created_at_ms:i64,pub completed_at_ms:Option<i64>,
    pub status:String,pub needs_response:bool,pub is_current:bool,
}
#[derive(Debug,Serialize)]
pub struct ActivityPage { pub entries:Vec<ActivityEntry>,pub next_cursor:Option<ActivityCursor> }
#[derive(Debug,Serialize)]
pub struct ActivityDetail {
    pub entry:ActivityEntry,
    pub explanation:Option<String>,
    pub result_text:Option<String>,
    pub result_has_more:bool,
    pub source_conversation_id:Option<String>,
    /// Navigation only. The source conversation revalidates all operations.
    pub review_in_source:bool,
    /// Exact recorded source Host only; absence never means the observing device.
    pub execution_host_id:Option<String>,
    /// Presentation scope derived from canonical conversation ownership.
    pub context_scope:String,
}
fn entry(row:ActivityRecord)->Result<ActivityEntry,String> {
    let status=if row.run_status=="sealed" {
        if row.seal_sources!=1 {return Err("Activity requires exactly one committed terminal source".into());}
        let seal:AssistantRunTerminalSeal=serde_json::from_str(row.seal_json.as_deref().ok_or("Activity sealed Run has no committed seal")?).map_err(|e|e.to_string())?;
        seal.validate()?;
        if seal.identity.run_id!=row.run_id || seal.identity.conversation_id!=row.conversation_id
            || seal.identity.assistant_message_id!=row.assistant_message_id || Some(seal.completed_at_ms)!=row.completed_at_ms {
            return Err("Activity terminal ownership mismatch".into());
        }
        if let Some(raw)=&row.result_json {
            let result:nola_domain::run_result::RunResult=serde_json::from_str(raw).map_err(|e|e.to_string())?;
            if result!=seal.result {return Err("Activity result differs from its seal".into());}
        }
        serde_json::to_value(&seal.result.value).map_err(|e|e.to_string())?["kind"].as_str().ok_or("Activity result kind missing")?.to_owned()
    } else {row.run_status};
    Ok(ActivityEntry {run_id:row.run_id,conversation_id:row.conversation_id,assistant_message_id:row.assistant_message_id,
        source:row.source,title:row.title,created_at_ms:row.created_at_ms,completed_at_ms:row.completed_at_ms,
        status,needs_response:row.needs_response,is_current:row.is_current})
}
pub fn list<R:ActivityRepository+ConsistentReadRepository>(store:&mut R,input:ActivityQuery)->Result<ActivityPage,String> {
    if !(1..=100).contains(&input.limit) || input.query.len()>512 || input.source.as_ref().is_some_and(|s|!matches!(s.as_str(),"agent"|"automation"|"conversation")) {
        return Err("invalid Activity query".into());
    }
    store.with_consistent_read(|store| {
        let mut rows=store.activity_records(&input).map_err(|e|e.to_string())?;
        let more=rows.len()>input.limit;rows.truncate(input.limit);
        let entries=rows.into_iter().map(entry).collect::<Result<Vec<_>,_>>()?;
        let next_cursor=if more {entries.last().map(|e|ActivityCursor{created_at_ms:e.created_at_ms,run_id:e.run_id.clone()})} else {None};
        Ok(ActivityPage{entries,next_cursor})
    })
}
pub fn detail<R:ActivityRepository+ConsistentReadRepository+RemoteSessionRepository+nola_store_port::conversation_history::ConversationProjectionRepository>(store:&mut R,conversation_id:String,run_id:String,assistant_message_id:String)->Result<Option<ActivityDetail>,String> {
    store.with_consistent_read(|store| {
        let Some(row)=store.activity_record(&conversation_id,&run_id).map_err(|e|e.to_string())? else{return Ok(None)};
        if row.assistant_message_id!=assistant_message_id {return Err("Activity assistant owner changed".into());}
        let execution_host_id=row.execution_host_id.clone();
        let entry=entry(row)?;
        let terminal=store.canonical_run_terminal(&conversation_id,&run_id).map_err(|e|e.to_string())?;
        if entry.completed_at_ms.is_some() && terminal.is_none() {return Err("Activity terminal is unavailable".into());}
        let explanation=terminal.as_ref().and_then(|terminal|match &terminal.result.value {
            nola_domain::run_result::RunResultValue::Blocked{reason,..}
            | nola_domain::run_result::RunResultValue::Incomplete{reason,..} => Some(reason.detail.clone().unwrap_or_else(||reason.code.clone())),
            _ => None,
        });
        let explanation=match terminal.as_ref().map(|terminal| &terminal.result.value) {
            Some(nola_domain::run_result::RunResultValue::Failed{error_ref,..}) =>
                store.activity_error_text(&conversation_id,&run_id,error_ref).map_err(|e|e.to_string())?,
            _ => explanation,
        };
        let (result_text,result_has_more)=match terminal.as_ref().map(|t|&t.result.value) {
            Some(nola_domain::run_result::RunResultValue::Answered{answer_part_refs}) =>
                store.activity_result_text(&conversation_id,&run_id,answer_part_refs).map_err(|e|e.to_string())?,
            _ => (None,false),
        };
        let (source_conversation_id,context_scope)=match store.conversation_execution_ownership(&conversation_id).map_err(|e|e.to_string())? {
            nola_domain::ConversationExecutionOwnership::User => (Some(conversation_id),"conversation"),
            nola_domain::ConversationExecutionOwnership::TaskChild{parent_conversation_id,..} => (Some(parent_conversation_id),"task"),
            nola_domain::ConversationExecutionOwnership::Unavailable => (None,"unavailable"),
        };
        let review_in_source=entry.is_current && source_conversation_id.is_some();
        Ok(Some(ActivityDetail{entry,explanation,result_text,result_has_more,source_conversation_id,review_in_source,execution_host_id,context_scope:context_scope.into()}))
    })
}
