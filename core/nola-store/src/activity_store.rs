// SPDX-License-Identifier: Apache-2.0
//! One bounded join supplies history; no per-row terminal/trace queries.
use super::*;
use nola_store_port::activity::*;
const READ: &str = "WITH activity AS (
 SELECT r.id run_id,r.conversation_id,r.assistant_message_id,
 CASE WHEN ab.run_id IS NOT NULL THEN 'automation' WHEN tb.run_id IS NOT NULL THEN 'agent' ELSE 'conversation' END source,
 COALESCE(ad.name,t.title,c.title) title,r.created_at_ms,r.completed_at_ms,r.status,
 (r.status<>'sealed' AND (r.status='waiting_input'
  OR EXISTS(SELECT 1 FROM user_input_wait_heads w WHERE w.run_id=r.id AND w.conversation_id=r.conversation_id AND w.state='open')
  OR EXISTS(SELECT 1 FROM delegated_invocations d WHERE d.run_id=r.id AND d.state='waiting_permission')
  OR EXISTS(SELECT 1 FROM permission_requests p WHERE p.child_session_id=r.conversation_id AND p.status='pending')
  OR EXISTS(SELECT 1 FROM core_checkpoints cp JOIN core_checkpoint_bodies cb ON cb.conversation_id=cp.conversation_id
    JOIN committed_transitions ct ON ct.run_id=r.id AND ct.checkpoint_revision=cp.revision
    WHERE cp.conversation_id=r.conversation_id AND json_array_length(cb.checkpoint_json,'$.state.execution.permissions.pending')>0))) needs_response,
 rr.result_json,COALESCE(s.seal_json,rs.seal_json) seal_json,
 (c.title||' '||COALESCE(ad.name,'')||' '||COALESCE(t.title,'')||' '||
 CASE WHEN ab.run_id IS NOT NULL THEN 'automation' WHEN tb.run_id IS NOT NULL THEN 'agent' ELSE 'conversation' END||' '||
 COALESCE(json_extract(COALESCE(rr.result_json,json_extract(COALESCE(s.seal_json,rs.seal_json),'$.result')),'$.value.kind'),r.status)||' '||
 COALESCE(json_extract(COALESCE(rr.result_json,json_extract(COALESCE(s.seal_json,rs.seal_json),'$.result')),'$.value.reason.detail'),'')||' '||
 COALESCE(json_extract(COALESCE(rr.result_json,json_extract(COALESCE(s.seal_json,rs.seal_json),'$.result')),'$.value.reason.code'),'')) search_text,
 (r.status<>'sealed' AND NOT EXISTS(SELECT 1 FROM runs newer WHERE newer.conversation_id=r.conversation_id
   AND newer.kind='conversation' AND newer.created_at_ms>r.created_at_ms)) is_current,((s.run_id IS NOT NULL)+(rs.run_id IS NOT NULL)) seal_sources,src.source_host_id execution_host_id
 FROM runs r JOIN conversations c ON c.id=r.conversation_id
 JOIN messages m ON m.id=r.assistant_message_id AND m.conversation_id=r.conversation_id AND m.run_id=r.id AND m.role='assistant'
 LEFT JOIN remote_run_sources src ON src.run_id=r.id
 LEFT JOIN automation_execution_bindings ab ON ab.run_id=r.id
 LEFT JOIN automation_runs ar ON ar.id=ab.occurrence_id
 LEFT JOIN automation_definitions ad ON ad.id=ar.automation_id AND ad.revision=ar.definition_revision
 LEFT JOIN runtime_task_run_bindings tb ON tb.run_id=r.id
 LEFT JOIN durable_tasks t ON t.id=tb.task_id
 LEFT JOIN run_results rr ON rr.run_id=r.id
 LEFT JOIN assistant_run_terminal_seals s ON s.run_id=r.id AND s.conversation_id=r.conversation_id
 LEFT JOIN remote_assistant_run_terminal_stages rs ON rs.run_id=r.id AND rs.committed=1 AND rs.manifest_complete=1
 WHERE r.kind='conversation'
) SELECT run_id,conversation_id,assistant_message_id,source,title,created_at_ms,completed_at_ms,status,
 needs_response,result_json,seal_json,is_current,seal_sources,execution_host_id FROM activity ";
fn row(r: &Row<'_>) -> rusqlite::Result<ActivityRecord> {
    Ok(ActivityRecord { run_id:r.get(0)?,conversation_id:r.get(1)?,assistant_message_id:r.get(2)?,source:r.get(3)?,
        title:r.get(4)?,created_at_ms:r.get(5)?,completed_at_ms:r.get(6)?,run_status:r.get(7)?,needs_response:r.get(8)?,
        result_json:r.get(9)?,seal_json:r.get(10)?,is_current:r.get(11)?,seal_sources:r.get(12)?,execution_host_id:r.get(13)? })
}
impl ActivityRepository for NolaStore {
    type Error=StoreError;
    fn activity_result_text(&self,conversation:&str,run:&str,parts:&[String])->StoreResult<(Option<String>,bool)> {
        crate::assistant_journal::activity_result_text(&self.connection,conversation,run,parts)
    }
    fn activity_error_text(&self,conversation:&str,run:&str,error_ref:&str)->StoreResult<Option<String>> {
        crate::run_requirements::activity_error_text(&self.connection,conversation,run,error_ref)
    }
    fn activity_records(&self,q:&ActivityQuery)->StoreResult<Vec<ActivityRecord>> {
        let sql=format!("{READ} WHERE (?1 IS NULL OR source=?1) AND (?2=0 OR needs_response=1)
          AND (?3='' OR instr(lower(search_text),lower(?3))>0)
          AND (?4 IS NULL OR created_at_ms<?4 OR (created_at_ms=?4 AND run_id<?5))
          ORDER BY created_at_ms DESC,run_id DESC LIMIT ?6");
        let mut stmt=self.connection.prepare(&sql)?;
        let rows=stmt.query_map(params![q.source,q.needs_response,q.query,q.cursor.as_ref().map(|c|c.created_at_ms),
            q.cursor.as_ref().map(|c|c.run_id.as_str()),q.limit as i64+1],row)?.collect::<Result<Vec<_>,_>>()?;
        Ok(rows)
    }
    fn activity_record(&self,conversation:&str,run:&str)->StoreResult<Option<ActivityRecord>> {
        self.connection.query_row(&format!("{READ} WHERE conversation_id=?1 AND run_id=?2"),params![conversation,run],row).optional().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_is_keyset_paginated_and_uses_real_source_bindings() {
        let store=NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Activity fixture',1,1,1);
          INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
            VALUES('a1','c','r1','assistant','streaming',1,1),('a2','c','r2','assistant','streaming',2,2);
          INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('r1','c','conversation','a1','running',1,1,1),('r2','c','conversation','a2','waiting_input',1,2,2);
          INSERT INTO runtime_task_run_bindings(task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,bound_at_ms)
            VALUES('task','owner','c','r2','attempt',2);").unwrap();
        store.connection.execute("INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('r2','host-fixture')", []).unwrap();
        let mut q=ActivityQuery{source:None,needs_response:false,query:"".into(),cursor:None,limit:1};
        let rows=store.activity_records(&q).unwrap();
        assert_eq!(rows[0].execution_host_id.as_deref(),Some("host-fixture"));
        assert!(rows[1].execution_host_id.is_none(), "missing provenance must not become the observing Host");
        assert_eq!(rows.len(),2);assert_eq!(rows[0].source,"agent");assert!(rows[0].needs_response);
        q.cursor=Some(ActivityCursor{created_at_ms:rows[0].created_at_ms,run_id:rows[0].run_id.clone()});
        assert_eq!(store.activity_records(&q).unwrap()[0].run_id,"r1");
        q.cursor=None;q.source=Some("automation".into());assert!(store.activity_records(&q).unwrap().is_empty());
        assert!(store.activity_record("foreign","r2").unwrap().is_none());
        q.source=None;q.needs_response=true;assert_eq!(store.activity_records(&q).unwrap().len(),1);
        q.needs_response=false;q.query="agent".into();assert_eq!(store.activity_records(&q).unwrap().len(),1);
        q.query="fixture".into();assert_eq!(store.activity_records(&q).unwrap().len(),2);
    }
}
