// SPDX-License-Identifier: Apache-2.0
//! Same-connection adapters; no new transaction or read visibility policy.
use crate::{NolaStore, StoreError, StoreResult, OptionalExtension};
use nola_store_port::conversation_history::*;
impl CanonicalHistoryRepository for NolaStore {
    fn load_conversation_message_window(
        &self,
        id: &str,
        before: Option<i64>,
        limit: usize,
        bytes: usize,
    ) -> StoreResult<Option<StoredConversationMessageWindow>> {
        NolaStore::load_conversation_message_window(self, id, before, limit, bytes)
    }
}
impl ConversationProjectionRepository for NolaStore {
    type ProjectionError = StoreError;
    fn conversation_source_owner(&self, id: &str) -> StoreResult<nola_domain::ConversationSourceOwner> {
        use nola_domain::ConversationSourceOwner;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)", [id], |row| row.get(0))?;
        if !exists { return Ok(ConversationSourceOwner::Unavailable); }
        let marker_json:Option<String>=self.connection.query_row("SELECT value_json FROM product_settings WHERE key=?1",[nola_domain::remote_expert_parent::source_key(id)],|r|r.get(0)).optional()?;
        let marker=marker_json.map(|raw|serde_json::from_str::<nola_domain::remote_expert_parent::RemoteExpertParentSource>(&raw)).transpose()?;
        if let Some(marker)=&marker {marker.validate(id).map_err(|e|StoreError::Invariant(e.into()))?;}
        let context_json:Option<String>=self.connection.query_row("SELECT value_json FROM product_settings WHERE key=?1",[nola_domain::remote_expert_parent::context_source_key(id)],|r|r.get(0)).optional()?;
        let context=context_json.map(|raw|serde_json::from_str::<nola_domain::remote_expert_parent::RemoteExpertContextSource>(&raw)).transpose()?;
        if let Some(context)=&context {
            context.validate(id).map_err(|e|StoreError::Invariant(e.into()))?;
            let parent_marker:Option<String>=self.connection.query_row("SELECT value_json FROM product_settings WHERE key=?1",[nola_domain::remote_expert_parent::source_key(&context.parent_conversation_id)],|r|r.get(0)).optional()?;
            if marker.is_some() || parent_marker.is_none() || self.conversation_source_owner(&context.parent_conversation_id)?!=context.owner(){return Ok(ConversationSourceOwner::Unavailable);}
        }
        let incomplete: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_assistant_journal_sources j JOIN runs r ON r.id=j.run_id
             WHERE r.conversation_id=?1 AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id AND s.source_host_id=j.source_host_id))
             OR EXISTS(SELECT 1 FROM remote_conversation_execution_owners o JOIN runs r ON r.id=o.run_id
             WHERE r.conversation_id=?1 AND NOT EXISTS(SELECT 1 FROM remote_assistant_journal_sources j
             WHERE j.run_id=r.id AND j.source_host_id=o.source_host_id AND j.source_store_epoch=o.source_store_epoch))",
            [id], |row| row.get(0))?;
        if incomplete { return Ok(ConversationSourceOwner::Unavailable); }
        // A source is a durable conversation fact, not whichever model/Host is
        // selected now. Per-Run rows are immutable but not unique per conversation.
        let mut query = self.connection.prepare_cached(
            "SELECT DISTINCT s.source_host_id,j.source_store_epoch FROM runs r
             JOIN remote_run_sources s ON s.run_id=r.id
             LEFT JOIN remote_assistant_journal_sources j ON j.run_id=r.id AND j.source_host_id=s.source_host_id
             WHERE r.conversation_id=?1 AND r.kind='conversation'")?;
        let owners = query.query_map([id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)))?
            .collect::<Result<Vec<_>,_>>()?;
        let resolved = match owners.as_slice() {
            [] => Ok(ConversationSourceOwner::Local),
            [(host, Some(epoch))] if !host.trim().is_empty() && !epoch.trim().is_empty() =>
                Ok(ConversationSourceOwner::Remote { source_host_id: host.clone(), source_store_epoch: epoch.clone() }),
            _ => Ok(ConversationSourceOwner::Unavailable),
        };
        if let Some(context)=context {
            let has_local:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runs r WHERE r.conversation_id=?1 AND r.kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",[id],|r|r.get(0))?;
            return Ok(if !has_local && (owners.is_empty() || resolved?==context.owner()){context.owner()}else{ConversationSourceOwner::Unavailable});
        }
        if let Some(marker)=marker {
            let has_local:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runs r WHERE r.conversation_id=?1 AND r.kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",[id],|r|r.get(0))?;
            if has_local {return Ok(ConversationSourceOwner::Unavailable);}
            let resolved=resolved?;
            return Ok(if owners.is_empty() || resolved==marker.owner(){marker.owner()}else{ConversationSourceOwner::Unavailable});
        }
        resolved
    }

    fn conversation_execution_ownership(&self, id: &str) -> StoreResult<nola_domain::ConversationExecutionOwnership> {
        use nola_domain::ConversationExecutionOwnership;
        // Mirrors lack source task admission authority. A missing local Task
        // must never be interpreted as permission to create an ordinary Run.
        if self.has_remote_presentation_owner(id)? || self.has_active_remote_run(id)? {
            let raw: Option<String> = self.connection.query_row(
                "SELECT o.ownership_json FROM remote_conversation_execution_owners o JOIN runs r ON r.id=o.run_id JOIN remote_assistant_journal_sources s ON s.run_id=r.id AND s.source_host_id=o.source_host_id AND s.source_store_epoch=o.source_store_epoch WHERE r.id=(SELECT id FROM runs WHERE conversation_id=?1 AND kind='conversation' ORDER BY created_at_ms DESC,rowid DESC LIMIT 1)",
                [id], |row| row.get(0)).optional()?;
            return raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
                .unwrap_or(Ok(ConversationExecutionOwnership::Unavailable));
        }
        source_execution_ownership(&self.connection, id)
    }

    fn conversation_cache_usage(&self, id: &str) -> StoreResult<Option<nola_domain::ConversationCacheUsage>> {
        NolaStore::conversation_cache_usage(self, id)
    }
    fn conversation_committed_context_usage(&self, id: &str) -> StoreResult<Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>> {
        NolaStore::conversation_committed_context_usage(self, id)
    }
    fn load_conversation(&self, id: &str) -> StoreResult<Option<StoredConversation>> {
        NolaStore::load_conversation(self, id)
    }
    fn load_conversation_page(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> StoreResult<Option<StoredConversation>> {
        NolaStore::load_conversation_page(self, id, offset, limit)
    }
    fn load_local_presentation_inputs(&self, id: &str, ids: &str, current: Option<&nola_store_port::run_effect::StoredCheckpoint>) -> StoreResult<Option<String>> {
        NolaStore::load_local_presentation_inputs(self, id, ids, current)
    }
    fn has_remote_presentation_owner(&self, id: &str) -> StoreResult<bool> {
        NolaStore::has_remote_presentation_owner(self, id)
    }
    fn has_active_remote_run(&self, id: &str) -> StoreResult<bool> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runs r JOIN remote_run_sources s ON s.run_id = r.id
             WHERE r.conversation_id = ?1 AND r.kind = 'conversation'
               AND r.status<>'sealed')",
                [id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
    fn delegated_text_part_owner(
        &self,
        id: &str,
        part: &str,
    ) -> StoreResult<Option<DelegatedTextOwner>> {
        NolaStore::delegated_text_part_owner(self, id, part).map(|owner| {
            owner.map(|o| DelegatedTextOwner {
                run_id: o.run_id,
                invocation_id: o.invocation_id,
            })
        })
    }
    fn model_text_part_owner(&self, id: &str, part: &str) -> StoreResult<Option<ModelTextOwner>> {
        NolaStore::model_text_part_owner(self, id, part).map(|owner| {
            owner.map(|o| ModelTextOwner {
                run_id: o.run_id,
                attempt_id: o.attempt_id,
            })
        })
    }
}

/// Source classification is derived from existing immutable Task admission.
/// It does not infer task identity from conversation names or message text.
pub(crate) fn source_execution_ownership(connection: &crate::Connection, id: &str) -> StoreResult<nola_domain::ConversationExecutionOwnership> {
    use nola_domain::ConversationExecutionOwnership;
    let mut statement = connection.prepare("SELECT id FROM durable_tasks WHERE kind='agent' AND child_session_id=?1 UNION SELECT task_id FROM runtime_task_run_bindings WHERE child_session_id=?1")?;
    let ids = statement.query_map([id], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let mut persistent_expert=false;
    for task_id in ids {
        if crate::expert_task_target::validated_expert_task_target(connection,&task_id,id,false)?.is_some() {
            persistent_expert=true;
            continue;
        }
        let Some(authority) = crate::agent_task_admission_store::authority(connection, &task_id)? else {
            return Ok(ConversationExecutionOwnership::Unavailable);
        };
        let Some(admission) = authority.admission else { return Ok(ConversationExecutionOwnership::Unavailable); };
        admission.validate().map_err(StoreError::Invariant)?;
        if admission.child_conversation_id != id { return Err(StoreError::Invariant("Task child admission differs from conversation".into())); }
        return Ok(ConversationExecutionOwnership::TaskChild {
            parent_conversation_id: admission.parent_conversation_id, parent_run_id: admission.parent_run_id,
            task_id, child_conversation_id: admission.child_conversation_id,
        });
    }
    if persistent_expert {return Ok(ConversationExecutionOwnership::User);}
    let bound: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE child_session_id=?1)", [id], |row| row.get(0))?;
    Ok(if bound { ConversationExecutionOwnership::Unavailable } else { ConversationExecutionOwnership::User })
}

#[cfg(test)]
mod source_owner_tests {
    use super::*;
    use nola_domain::ConversationSourceOwner;

    #[test]
    fn source_owner_is_durable_and_rejects_conflicting_run_sources() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source-owner.sqlite");
        let store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,configuration_revision,created_at_ms,updated_at_ms) VALUES('conversation','Fixture',1,1,1,1);").unwrap();
        assert_eq!(store.conversation_source_owner("missing").unwrap(), ConversationSourceOwner::Unavailable);
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), ConversationSourceOwner::Local);
        store.connection.execute_batch("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('first','conversation','conversation','running',1,1,1);
            INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('first','host-a');").unwrap();
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), ConversationSourceOwner::Unavailable);
        store.connection.execute_batch("INSERT INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES('first','host-a','epoch-a','assistant-a');").unwrap();
        let expected = ConversationSourceOwner::Remote { source_host_id:"host-a".into(),source_store_epoch:"epoch-a".into() };
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), expected);
        store.connection.execute("DELETE FROM remote_run_sources WHERE run_id='first'", []).unwrap();
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), ConversationSourceOwner::Unavailable,
            "journal-only ownership cannot become a writable local conversation");
        store.connection.execute("INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('first','host-a')", []).unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), expected);
        store.connection.execute_batch("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('second','conversation','conversation','running',1,2,2);
            INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('second','host-b');
            INSERT INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES('second','host-b','epoch-b','assistant-b');").unwrap();
        assert_eq!(store.conversation_source_owner("conversation").unwrap(), ConversationSourceOwner::Unavailable,
            "a newer Run cannot silently replace the conversation source authority");
    }
}
