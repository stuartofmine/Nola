// SPDX-License-Identifier: Apache-2.0
//! The production window/fresh projection pipeline runs without a SQLite Store.
use nola_domain::{CanonicalPartOwner, HistoricalPartProjection, HistoricalPartRead};
use nola_store_port::conversation_history::*;
use nola_store_port::historical_part_projection::HistoricalPartProjectionRepository;
use nola_store_port::protocol_journal::*;
use nola_store_port::run_effect::*;
use std::cell::Cell;

struct MemoryHistory {
    reads: Cell<usize>,
    stale: bool,
}
impl MemoryHistory {
    fn conversation(&self) -> StoredConversation {
        StoredConversation {
            configuration_revision: 1,
            id: "c".into(),
            title: "Port history".into(),
            revision: 2,
            created_at_ms: 1,
            updated_at_ms: 2,
            settings_json: None,
            messages: vec![StoredMessage {
                id: "m".into(),
                run_id: None,
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: (0..64)
                    .map(|i| StoredPart {
                        id: format!("p{i}"),
                        kind: "text".into(),
                        ordinal: i + 1,
                        state: "completed".into(),
                        content_text: String::new(),
                        content_json: None,
                        content_json_deferred: false,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    })
                    .collect(),
            }],
        }
    }
}
impl HistoricalPartProjectionRepository for MemoryHistory {
    type Error = String;
    fn read_historical_part(
        &self,
        _: &CanonicalPartOwner,
        _: i64,
        _: usize,
    ) -> Result<Option<HistoricalPartProjection>, String> {
        panic!("production must use typed budget read")
    }
    fn read_historical_part_with_budget(
        &self,
        owner: &CanonicalPartOwner,
        revision: i64,
        budget: usize,
    ) -> Result<HistoricalPartRead, String> {
        if self.stale
            || owner.conversation_id() != "c"
            || owner.message_id() != "m"
            || revision != 2
        {
            return Err("stale canonical source".into());
        }
        self.reads.set(self.reads.get() + 1);
        let value: HistoricalPartProjection=serde_json::from_value(serde_json::json!({
            "owner":owner,"canonical_revision":2,"canonical_kind":"text","source":null,"schema":"message_part_view","binding":"canonical",
            "status":{"state":"supported"},"fields":{"markdown":{"availability":"complete","value":"原".repeat(100)}}
        })).unwrap();
        Ok(if serde_json::to_vec(&value).unwrap().len() <= budget {
            HistoricalPartRead::Available(Box::new(value))
        } else {
            HistoricalPartRead::BudgetDeferred {
                canonical_kind: "text".into(),
            }
        })
    }
}
impl nola_store_port::tool_source::ToolSourceRepository for MemoryHistory {
    fn tool_sources_for_part(&self,_:&CanonicalPartOwner,_:i64)->Result<Vec<nola_store_port::tool_source::ToolSourceBinding>,String> {
        Err("text-only fixture has no tool Parts".into())
    }
    fn tool_source_by_id(&self,_:&str,_:&str,_:i64)->Result<Option<nola_store_port::tool_source::ToolSourceBinding>,String> {
        Err("text-only fixture has no tool sources".into())
    }
    fn read_tool_source_range(&self,_:&nola_store_port::tool_source::ToolSourceBinding,_:i64,_:u64,_:usize)->Result<nola_store_port::tool_source::ToolSourceRange,String> {
        Err("text-only fixture has no tool sources".into())
    }
}
impl CanonicalHistoryRepository for MemoryHistory {
    fn load_conversation_message_window(
        &self,
        id: &str,
        before: Option<i64>,
        limit: usize,
        bytes: usize,
    ) -> Result<Option<StoredConversationMessageWindow>, String> {
        assert_eq!(id, "c");
        assert_eq!(before, None);
        assert_eq!(limit, 32);
        assert_eq!(bytes, 32768);
        Ok(Some(StoredConversationMessageWindow {
            assistant_journal_heads: vec![],
            conversation: self.conversation(),
            total_message_count: 1,
            before_ordinal: Some(1),
            has_more: false,
            deferred_text_parts: vec![],
        }))
    }
}
impl RunEffectRepository for MemoryHistory {
    type Error = String;
    fn prepare_unstarted_cancellation_target(&self, _: &str,
        _: &nola_domain::host_control::HostEffectControl, _: &nola_domain::host_control::HostEffectControl,
        _: i64, _: i64) -> Result<Option<nola_store_port::run_effect::PreparedCancellationNotStarted>,String> {
        unreachable!("historical reader cannot revoke execution admission")
    }
    fn pending_outbox_effect_ids(&self, _: &str) -> Result<Vec<String>, String> {
        panic!("historical projection must not read execution settlement inventory")
    }
    fn committed_terminal_host_response(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Option<nola_store_port::run_effect::CommittedHostResponseReceipt>, String> {
        Ok(None)
    }
    fn checkpoint_replay_pending(&self, _: &str) -> Result<bool, String> { Ok(false) }
    fn checkpoint_replay_ready(&self, _: &str, _: &str, _: i64, _: usize) -> Result<bool, String> { unreachable!("history does not write checkpoints") }
    fn load_recovery_checkpoint(&self, _: &str) -> Result<Option<nola_store_port::run_effect::StoredRecoveryCheckpoint>, String> {
        unreachable!("history reads do not restore execution")
    }
    fn append_host_event_history(&mut self, _: &str, _: &[nola_store_port::run_effect::HostEventInput]) -> Result<(), String> {
        panic!("history reads cannot append execution inputs")
    }
    fn unrecorded_memory_usage_entries(&self, _: &[nola_store_port::memory::StoredMemoryUsageEntry])
        -> Result<Vec<nola_store_port::memory::StoredMemoryUsageEntry>, String> {
        panic!("historical projection must not plan accounting mutations")
    }
    fn load_canonical_text_state(
        &self,
        _: &nola_domain::CanonicalStreamOwner,
    ) -> Result<nola_domain::CanonicalTextState, String> {
        panic!("historical projection must not plan text mutations")
    }
    fn delegated_permission(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        panic!("unexpected permission ledger read")
    }
    fn delegated_permission_for_occurrence(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        panic!("unexpected permission ledger read")
    }
    fn delegated_permission_for_control(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        panic!("unexpected permission ledger read")
    }
    fn open_delegated_permissions_for_target(
        &self,
        _: &str,
    ) -> Result<Vec<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        panic!("unexpected permission ledger read")
    }
    fn delegated_invocation(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        panic!("unexpected execution ledger read")
    }
    fn delegated_invocation_for_effect(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        panic!("unexpected execution ledger read")
    }
    fn model_attempt(&self, _: &str) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        panic!("historical projection must not load model execution state")
    }
    fn active_model_attempt_for_effect(
        &self,
        _: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        panic!("historical projection must not load model execution state")
    }
    fn active_run(&self, _: &str) -> Result<Option<StoredActiveRun>, String> {
        Ok(None)
    }
    fn checkpoint_revision(&self, _: &str) -> Result<Option<i64>, String> {
        panic!("no checkpoint fallback")
    }
    fn load_checkpoint(&self, _: &str) -> Result<Option<StoredCheckpoint>, String> {
        panic!("no checkpoint fallback")
    }
    fn load_run_invocation_config(
        &self,
        _: &str,
    ) -> Result<Option<StoredRunInvocationConfig>, String> {
        unreachable!()
    }
    fn outbox_effect(&self, _: &str) -> Result<Option<StoredOutboxEffect>, String> {
        unreachable!()
    }
    fn outbox_effect_identity(&self, _: &str) -> Result<Option<StoredOutboxEffectIdentity>, String> {
        unreachable!()
    }
    fn load_committed_host_delivery(
        &self,
        _: &str,
    ) -> Result<CommittedHostDeliverySnapshot, String> {
        unreachable!()
    }
    fn load_committed_host_delivery_witness(
        &self, _: &str,
    ) -> Result<CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>, String> {
        unreachable!("history reads do not admit delivery")
    }
    fn load_effect_heartbeat_snapshot(
        &self, _: &str, _: &str,
    ) -> Result<Option<nola_store_port::run_effect::StoredEffectHeartbeatSnapshot>, String> {
        unreachable!("history reads do not renew execution leases")
    }
    fn load_selected_host_delivery_witness(
        &self, _: &str, _: Option<&std::collections::HashSet<String>>,
    ) -> Result<CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>, String> {
        unreachable!("history reads do not claim effects")
    }
    fn commit_transition(
        &mut self,
        _: &CommitTransitionInput,
    ) -> Result<CommitTransitionResult, String> {
        panic!("history query cannot write")
    }
}
impl ProjectionJournalRepository for MemoryHistory {
    type Error = String;
    fn append_projection_event(
        &mut self,
        _: &AppendProjectionEventInput,
    ) -> Result<StoredProjectionEvent, String> {
        panic!("history query cannot write")
    }
    fn projection_cursor(
        &self,
        _: &ProjectionScopeKey,
    ) -> Result<Option<StoredProjectionCursor>, String> {
        Ok(None)
    }
    fn projection_events_after(
        &self,
        _: &ProjectionScopeKey,
        _: u64,
        _: usize,
    ) -> Result<Vec<StoredProjectionEvent>, String> {
        unreachable!()
    }
}
impl ConversationProjectionRepository for MemoryHistory {
    fn conversation_source_owner(&self, id: &str) -> Result<nola_domain::ConversationSourceOwner, String> {
        Ok(if self.conversation().id == id { nola_domain::ConversationSourceOwner::Local } else { nola_domain::ConversationSourceOwner::Unavailable })
    }
    fn conversation_execution_ownership(&self, _: &str) -> Result<nola_domain::ConversationExecutionOwnership, Self::ProjectionError> {
        Ok(nola_domain::ConversationExecutionOwnership::User)
    }

    type ProjectionError = String;
    fn conversation_cache_usage(&self, _: &str) -> Result<Option<nola_domain::ConversationCacheUsage>, String> { Ok(None) }
    fn conversation_committed_context_usage(&self, _: &str) -> Result<Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>, String> { Ok(None) }
    fn load_conversation(&self, _: &str) -> Result<Option<StoredConversation>, String> {
        Ok(Some(self.conversation()))
    }
    fn load_conversation_page(
        &self,
        _: &str,
        _: usize,
        _: usize,
    ) -> Result<Option<StoredConversation>, String> {
        Ok(Some(self.conversation()))
    }
    fn load_local_presentation_inputs(&self, _: &str, _: &str, _: Option<&nola_store_port::run_effect::StoredCheckpoint>) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn has_remote_presentation_owner(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn has_active_remote_run(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn delegated_text_part_owner(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<nola_store_port::conversation_history::DelegatedTextOwner>, String> {
        unreachable!()
    }
    fn model_text_part_owner(&self, _: &str, _: &str) -> Result<Option<ModelTextOwner>, String> {
        unreachable!()
    }
}
#[test]
fn real_window_and_full_projection_use_typed_port_without_checkpoint_or_write() {
    let history = MemoryHistory {
        reads: Cell::new(0),
        stale: false,
    };
    let window =
        nola_application::load_conversation_projection_window_v1(&history, "c", None, 32, 32768)
            .unwrap();
    assert_eq!(history.reads.get(), 64);
    assert_eq!(window.projection.messages[0].parts.len(), 64);
    assert!(serde_json::to_vec(&window).unwrap().len() <= 2 * 1024 * 1024);
    assert!(window.projection.messages[0].parts.iter().any(|part|matches!(part,nola_domain::MessagePartView::Historical{projection,..} if matches!(projection.status,nola_domain::HistoricalProjectionStatus::NotLoaded{..}))));
    let full = nola_application::load_conversation_projection_v1(&history, "c").unwrap();
    assert_eq!(full.messages, window.projection.messages);
    assert_eq!(history.reads.get(), 128);
}
#[test]
fn stale_source_is_an_error_not_a_plaintext_or_checkpoint_fallback() {
    let history = MemoryHistory {
        reads: Cell::new(0),
        stale: true,
    };
    let error =
        nola_application::load_conversation_projection_window_v1(&history, "c", None, 32, 32768)
            .unwrap_err();
    assert!(error.to_string().contains("stale canonical source"));
}
