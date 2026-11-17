// SPDX-License-Identifier: Apache-2.0
//! Consolidated Historical Window & Projection suite:
//! non-SQLite port budget & stale source, 10MB deferred historical JSON,
//! branch derivation & unsupported nodes, exhausted window pagination,
//! escaped owner metadata, reviewed text disclosure, measured Rust allocation
//! bounds, and receipted window-update invalidation.

mod non_sqlite_port {
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
            let value: HistoricalPartProjection = serde_json::from_value(serde_json::json!({
                "owner":owner,"canonical_revision":2,"canonical_kind":"text","source":null,"schema":"message_part_view","binding":"canonical",
                "status":{"state":"supported"},"fields":{"markdown":{"availability":"complete","value":"原".repeat(100)}}
            }))
            .unwrap();
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
        fn tool_sources_for_part(
            &self,
            _: &CanonicalPartOwner,
            _: i64,
        ) -> Result<Vec<nola_store_port::tool_source::ToolSourceBinding>, String> {
            Err("text-only fixture has no tool Parts".into())
        }
        fn tool_source_by_id(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<Option<nola_store_port::tool_source::ToolSourceBinding>, String> {
            Err("text-only fixture has no tool sources".into())
        }
        fn read_tool_source_range(
            &self,
            _: &nola_store_port::tool_source::ToolSourceBinding,
            _: i64,
            _: u64,
            _: usize,
        ) -> Result<nola_store_port::tool_source::ToolSourceRange, String> {
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
            assert_eq!((id, before, limit, bytes), ("c", None, 32, 32768));
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
        fn prepare_unstarted_cancellation_target(
            &self,
            _: &str,
            _: &nola_domain::host_control::HostEffectControl,
            _: &nola_domain::host_control::HostEffectControl,
            _: i64,
            _: i64,
        ) -> Result<Option<PreparedCancellationNotStarted>, String> {
            unreachable!("historical reader cannot revoke execution admission")
        }
        fn pending_outbox_effect_ids(&self, _: &str) -> Result<Vec<String>, String> {
            panic!("historical projection must not read execution settlement inventory")
        }
        fn committed_terminal_host_response(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<CommittedHostResponseReceipt>, String> {
            Ok(None)
        }
        fn checkpoint_replay_pending(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        fn checkpoint_replay_ready(
            &self,
            _: &str,
            _: &str,
            _: i64,
            _: usize,
        ) -> Result<bool, String> {
            unreachable!("history does not write checkpoints")
        }
        fn load_recovery_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<StoredRecoveryCheckpoint>, String> {
            unreachable!("history reads do not restore execution")
        }
        fn append_host_event_history(
            &mut self,
            _: &str,
            _: &[HostEventInput],
        ) -> Result<(), String> {
            panic!("history reads cannot append execution inputs")
        }
        fn unrecorded_memory_usage_entries(
            &self,
            _: &[nola_store_port::memory::StoredMemoryUsageEntry],
        ) -> Result<Vec<nola_store_port::memory::StoredMemoryUsageEntry>, String> {
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
        fn outbox_effect_identity(
            &self,
            _: &str,
        ) -> Result<Option<StoredOutboxEffectIdentity>, String> {
            unreachable!()
        }
        fn load_committed_host_delivery(
            &self,
            _: &str,
        ) -> Result<CommittedHostDeliverySnapshot, String> {
            unreachable!()
        }
        fn load_committed_host_delivery_witness(
            &self,
            _: &str,
        ) -> Result<CommittedHostDeliverySnapshot<StoredDeliveryCheckpointWitness>, String> {
            unreachable!("history reads do not admit delivery")
        }
        fn load_effect_heartbeat_snapshot(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<StoredEffectHeartbeatSnapshot>, String> {
            unreachable!("history reads do not renew execution leases")
        }
        fn load_selected_host_delivery_witness(
            &self,
            _: &str,
            _: Option<&std::collections::HashSet<String>>,
        ) -> Result<CommittedHostDeliverySnapshot<StoredDeliveryCheckpointWitness>, String> {
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
        type ProjectionError = String;
        fn conversation_source_owner(
            &self,
            id: &str,
        ) -> Result<nola_domain::ConversationSourceOwner, String> {
            Ok(if self.conversation().id == id {
                nola_domain::ConversationSourceOwner::Local
            } else {
                nola_domain::ConversationSourceOwner::Unavailable
            })
        }
        fn conversation_execution_ownership(
            &self,
            _: &str,
        ) -> Result<nola_domain::ConversationExecutionOwnership, Self::ProjectionError> {
            Ok(nola_domain::ConversationExecutionOwnership::User)
        }
        fn conversation_cache_usage(
            &self,
            _: &str,
        ) -> Result<Option<nola_domain::ConversationCacheUsage>, String> {
            Ok(None)
        }
        fn conversation_committed_context_usage(
            &self,
            _: &str,
        ) -> Result<Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>, String>
        {
            Ok(None)
        }
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
        fn load_local_presentation_inputs(
            &self,
            _: &str,
            _: &str,
            _: Option<&StoredCheckpoint>,
        ) -> Result<Option<String>, String> {
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
        ) -> Result<Option<DelegatedTextOwner>, String> {
            unreachable!()
        }
        fn model_text_part_owner(&self, _: &str, _: &str) -> Result<Option<ModelTextOwner>, String> {
            unreachable!()
        }
    }

    #[test]
    fn non_sqlite_window_and_full_projection_obey_typed_port_and_stale_source_contract() {
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
        assert!(window.projection.messages[0].parts.iter().any(|part| {
            matches!(
                part,
                nola_domain::MessagePartView::Historical { projection, .. }
                    if matches!(projection.status, nola_domain::HistoricalProjectionStatus::NotLoaded { .. })
            )
        }));
        let full = nola_application::load_conversation_projection_v1(&history, "c").unwrap();
        assert_eq!(full.messages, window.projection.messages);
        assert_eq!(history.reads.get(), 128);

        let stale = MemoryHistory {
            reads: Cell::new(0),
            stale: true,
        };
        assert!(
            nola_application::load_conversation_projection_window_v1(&stale, "c", None, 32, 32768)
                .unwrap_err()
                .to_string()
                .contains("stale canonical source")
        );
    }
}

mod cutover_and_budget {
    use crate::support;
    use nola_agent_engine::agent::parts::{
        AgentPart, PartOwnership, ReasoningPart, TextPart, TextReviewState,
    };
    use nola_application::{load_conversation_projection_window_v1, ApplicationCommand};
    use nola_domain::{
        CanonicalBranchIntent, CanonicalBranchTarget, CanonicalPartOwner, HistoricalPartProjection,
        HistoricalProjectionStatus, MessagePartView, SemanticIdentityAssessment, SemanticValue,
    };
    use nola_protocol::HistoricalProjectionCommand;
    use nola_store::NolaStore;
    use serde_json::json;

    #[test]
    fn historical_window_cutover_pagination_budget_and_escaped_metadata_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history-suite.db");
        drop(NolaStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();

        // 1. Seed 10MB tool JSON ("large"), branchable reasoning ("branch-src"),
        //    exhausted 2x12 reasoning messages ("c"), and escaped owner metadata ("escaped").
        let large_body = json!({
            "kind":"tool","execution_source":{"kind":"model"},"part_id":"p-large","revision":1,
            "tool_call_id":"call","capability_id":"read","title":"Read historical output",
            "origin":{"kind":"builtin"},"state":"completed","input":{},
            "output":"x".repeat(10_000_000),"affected_resources":[],"artifacts":[]
        })
        .to_string();
        db.execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('large','Large',1,1,2);
             INSERT INTO messages VALUES('m-large','large',NULL,'user','completed',1,1,2);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES('p-large','m-large','tool',1,'completed','plain preview',?1,1,2)",
            [&large_body],
        )
        .unwrap();

        let branch_body = json!({
            "kind":"reasoning","part_id":"p-branch","revision":1,
            "title":"Historical reasoning","state":"completed","summary":"Historical evidence"
        })
        .to_string();
        db.execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('branch-src','Branch',1,1,2);
             INSERT INTO messages VALUES('m-branch','branch-src',NULL,'user','completed',1,1,2);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES('p-branch','m-branch','reasoning',1,'completed','plain preview',?1,1,2)",
            [&branch_body],
        )
        .unwrap();

        db.execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1)",
            [],
        )
        .unwrap();
        for message in 1..=2 {
            let id = format!("m{message}");
            db.execute(
                "INSERT INTO messages VALUES(?1,'c','historical-turn','assistant','completed',?2,1,1)",
                rusqlite::params![id, message],
            )
            .unwrap();
            for part in 1..=12 {
                let part_id = format!("p{message}-{part}");
                let raw = serde_json::to_string(&AgentPart::Reasoning(ReasoningPart {
                    id: part_id.clone(),
                    ownership: PartOwnership {
                        message_id: id.clone(),
                        turn_id: "historical-turn".into(),
                    },
                    title: "Historical reasoning".into(),
                    detail: format!("marker-{message}-{part} {}", "x".repeat(4000)),
                    is_complete: true,
                }))
                .unwrap();
                db.execute(
                    "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,'reasoning',?3,'completed','',?4,1,1)",
                    rusqlite::params![part_id, id, part, raw],
                )
                .unwrap();
            }
        }
        db.execute(
            "INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
             VALUES('historical-turn','c','conversation','m1','running',1,1,1,NULL)",
            [],
        )
        .unwrap();

        let escaped_conv = format!("c{}", "\"".repeat(127));
        let escaped_msg = format!("m{}", "\"".repeat(127));
        let escaped_part = format!("p{}", "\"".repeat(127));
        let escaped_kind = "\"".repeat(128);
        db.execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,'History',1,1,1)",
            [&escaped_conv],
        )
        .unwrap();
        db.execute(
            "INSERT INTO messages VALUES(?1,?2,NULL,'user','completed',1,1,1)",
            rusqlite::params![escaped_msg, escaped_conv],
        )
        .unwrap();
        db.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES(?1,?2,?3,1,'completed','','{}',1,1)",
            rusqlite::params![escaped_part, escaped_msg, escaped_kind],
        )
        .unwrap();

        let mut store = NolaStore::open(&path).unwrap();

        // Check 10MB deferred tool JSON ("large").
        let raw_large = store
            .load_conversation_message_window("large", None, 32, 1024)
            .unwrap()
            .unwrap();
        assert!(raw_large.conversation.messages[0].parts[0].content_json.is_none());
        let large_win =
            load_conversation_projection_window_v1(&store, "large", None, 32, 1024).unwrap();
        let MessagePartView::Historical {
            part_id,
            projection,
            ..
        } = &large_win.projection.messages[0].parts[0]
        else {
            panic!("original JSON must not degrade to preview text")
        };
        assert_eq!(part_id.0, "p-large");
        assert_eq!(projection.binding, Some(SemanticIdentityAssessment::Canonical));
        assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
        assert!(matches!(
            projection.fields.get("output"),
            Some(SemanticValue::Deferred { .. })
        ));
        assert!(serde_json::to_vec(&large_win).unwrap().len() < 32768);

        // Check branch derivation & unsupported node ("branch-src" -> "derived").
        store
            .create_conversation_branch(&CanonicalBranchIntent {
                operation_id: "branch".into(),
                source_conversation_id: "branch-src".into(),
                expected_source_revision: 1,
                through_message_id: "m-branch".into(),
                conversation: CanonicalBranchTarget {
                    id: "derived".into(),
                    title: "Derived".into(),
                    settings_json: None,
                    created_at_ms: 3,
                    updated_at_ms: 3,
                },
                committed_at_ms: 3,
            })
            .unwrap();
        let derived_win =
            load_conversation_projection_window_v1(&store, "derived", None, 32, 1024).unwrap();
        let MessagePartView::Historical {
            part_id: d_part,
            projection: d_proj,
            ..
        } = &derived_win.projection.messages[0].parts[0]
        else {
            panic!("branch must use typed historical node")
        };
        assert_ne!(d_part.0, "p-branch");
        assert_eq!(d_part.0, d_proj.owner.part_id());
        assert_eq!(d_proj.owner.conversation_id(), "derived");
        assert_eq!(
            d_proj.binding,
            Some(SemanticIdentityAssessment::VerifiedLocalDerivation)
        );
        assert_eq!(d_proj.status, HistoricalProjectionStatus::Supported);

        // Check exhausted window pagination ("c").
        let window = load_conversation_projection_window_v1(&store, "c", None, 2, 200000).unwrap();
        assert_eq!(window.projection.messages.len(), 2);
        assert_eq!(
            window
                .projection
                .messages
                .iter()
                .map(|m| m.parts.len())
                .sum::<usize>(),
            24
        );
        let mut available_bytes = 0;
        let mut pending = Vec::new();
        for message in &window.projection.messages {
            for part in &message.parts {
                let MessagePartView::Historical {
                    part_id,
                    projection,
                    ..
                } = part
                else {
                    panic!("lost historical node")
                };
                assert_eq!(projection.owner.part_id(), part_id.0);
                assert_eq!(projection.owner.message_id(), message.message_id.0);
                if matches!(projection.status, HistoricalProjectionStatus::NotLoaded { .. }) {
                    assert!(
                        projection.source.is_none()
                            && projection.binding.is_none()
                            && projection.schema.is_none()
                            && projection.fields.is_empty()
                    );
                    pending.push(projection.owner.clone());
                } else {
                    available_bytes += serde_json::to_vec(projection).unwrap().len();
                }
            }
        }
        assert!(!pending.is_empty() && available_bytes <= 32768);
        assert!(serde_json::to_vec(&window).unwrap().len() < 65536);

        let newest = load_conversation_projection_window_v1(&store, "c", None, 1, 200000).unwrap();
        assert_eq!(newest.projection.messages[0].message_id.0, "m2");
        assert!(newest.has_more);
        let older = load_conversation_projection_window_v1(
            &store,
            "c",
            newest.before_ordinal,
            1,
            200000,
        )
        .unwrap();
        assert_eq!(older.projection.messages[0].message_id.0, "m1");
        assert!(!older.has_more);
        drop(store);

        // Mutate p-branch to unsupported future_kind and verify node visibility.
        db.execute(
            "UPDATE message_parts SET content_json='{\"kind\":\"future_kind\",\"part_id\":\"p-branch\"}' WHERE id='p-branch'",
            [],
        )
        .unwrap();
        let store = NolaStore::open(&path).unwrap();
        let unsupported =
            load_conversation_projection_window_v1(&store, "branch-src", None, 32, 1024).unwrap();
        let MessagePartView::Historical { projection: u_proj, .. } =
            &unsupported.projection.messages[0].parts[0]
        else {
            panic!("unsupported original must remain visible")
        };
        assert!(matches!(
            u_proj.status,
            HistoricalProjectionStatus::Unsupported { .. }
        ));
        assert!(u_proj.fields.is_empty());
        drop(store);

        // On-demand ReadHistoricalPart recovery & escaped owner metadata budget.
        let mut runtime = support::open_runtime(&path).unwrap();
        for owner in pending {
            let cmd: ApplicationCommand = HistoricalProjectionCommand::ReadHistoricalPart {
                owner: owner.clone(),
                expected_conversation_revision: 1,
                maximum_output_bytes: 8192,
            }
            .into();
            let complete: HistoricalPartProjection =
                serde_json::from_value(runtime.execute(cmd).unwrap()).unwrap();
            assert_eq!(complete.owner, owner);
            assert_eq!(complete.status, HistoricalProjectionStatus::Supported);
            assert!(matches!(
                complete.fields.get("detail"),
                Some(SemanticValue::Complete { .. })
            ));
        }
        assert!(runtime
            .execute(
                HistoricalProjectionCommand::ReadHistoricalPart {
                    owner: CanonicalPartOwner::new("c".into(), "m2".into(), "p2-12".into())
                        .unwrap(),
                    expected_conversation_revision: 2,
                    maximum_output_bytes: 8192,
                }
                .into()
            )
            .is_err());
        assert!(runtime
            .execute(
                HistoricalProjectionCommand::ReadHistoricalPart {
                    owner: CanonicalPartOwner::new("c".into(), "m1".into(), "p2-12".into())
                        .unwrap(),
                    expected_conversation_revision: 1,
                    maximum_output_bytes: 8192,
                }
                .into()
            )
            .is_err());

        let escaped_owner =
            CanonicalPartOwner::new(escaped_conv, escaped_msg, escaped_part).unwrap();
        assert!(runtime
            .execute(
                HistoricalProjectionCommand::ReadHistoricalPart {
                    owner: escaped_owner.clone(),
                    expected_conversation_revision: 1,
                    maximum_output_bytes: 1024,
                }
                .into()
            )
            .is_err());
        let escaped_ok = runtime
            .execute(
                HistoricalProjectionCommand::ReadHistoricalPart {
                    owner: escaped_owner,
                    expected_conversation_revision: 1,
                    maximum_output_bytes: 8192,
                }
                .into(),
            )
            .unwrap();
        assert!(serde_json::to_vec(&escaped_ok).unwrap().len() <= 8192);
    }

    #[test]
    fn imported_reviewed_text_obeys_disclosure_in_windows_details_and_ranges() {
        for review in [
            None,
            Some(TextReviewState::Approved),
            Some(TextReviewState::Pending),
            Some(TextReviewState::Rejected),
            Some(TextReviewState::Interrupted),
        ] {
            let public = review.is_none() || review == Some(TextReviewState::Approved);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("reviewed-history.db");
            drop(NolaStore::open(&path).unwrap());
            let db = rusqlite::Connection::open(&path).unwrap();
            let text = "CANDIDATE_ONLY 中文🙂\n".repeat(2048);
            let raw = serde_json::to_string(&AgentPart::Text(TextPart {
                id: "p".into(),
                ownership: PartOwnership {
                    message_id: "m".into(),
                    turn_id: "history".into(),
                },
                text: text.clone(),
                is_complete: true,
                review,
            }))
            .unwrap();
            db.execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Review history',1,1,1);
                 INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);",
            )
            .unwrap();
            db.execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
                 VALUES('p','m','text',1,'completed',?1,?2,1,1)",
                rusqlite::params![text, raw],
            )
            .unwrap();
            drop(db);
            let mut store = NolaStore::open(&path).unwrap();
            assert!(load_conversation_projection_window_v1(&store, "c", None, 1, 512).is_err());
            store.begin_write_unit_of_work().unwrap();
            store.commit_write_unit_of_work().unwrap();
            assert!(store.load_checkpoint("c").unwrap().is_none());
            let window = load_conversation_projection_window_v1(&store, "c", None, 1, 512).unwrap();
            assert_eq!(window.deferred_text_parts.len(), 1);
            let reference = &window.deferred_text_parts[0];
            assert!(reference.is_final);
            assert_eq!(reference.can_read_original, public);
            if !public {
                assert!(!serde_json::to_string(&window)
                    .unwrap()
                    .contains("CANDIDATE_ONLY"));
            }
            drop(store);
            let mut runtime = support::open_runtime(&path).unwrap();
            let projection: HistoricalPartProjection = serde_json::from_value(
                runtime
                    .execute(
                        HistoricalProjectionCommand::ReadHistoricalPart {
                            owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into())
                                .unwrap(),
                            expected_conversation_revision: 1,
                            maximum_output_bytes: 8192,
                        }
                        .into(),
                    )
                    .unwrap(),
            )
            .unwrap();
            if !public {
                assert!(projection.canonical_text.is_none());
                assert!(!projection.fields.contains_key("text"));
                assert!(!serde_json::to_string(&projection)
                    .unwrap()
                    .contains("CANDIDATE_ONLY"));
            } else {
                assert!(projection
                    .canonical_text
                    .as_ref()
                    .is_some_and(|p| p.text.starts_with("CANDIDATE_ONLY")));
            }
            let range = runtime.execute(ApplicationCommand::ReadConversationTextRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                offset_bytes: 0,
                limit_bytes: 32,
            });
            assert_eq!(range.is_ok(), public, "range review={review:?}");
            if let Ok(range) = range {
                assert!(range["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("CANDIDATE_ONLY"));
            }
            let search = runtime
                .execute(ApplicationCommand::SearchConversationText {
                    conversation_id: "c".into(),
                    query: "CANDIDATE_ONLY".into(),
                    limit: 1,
                })
                .unwrap();
            assert_eq!(search.as_array().unwrap().len(), usize::from(public));
            drop(runtime);
            let cold = NolaStore::open(&path).unwrap();
            assert_eq!(
                cold.load_conversation_message_window("c", None, 1, 512)
                    .unwrap()
                    .unwrap()
                    .deferred_text_parts[0]
                    .can_read_original,
                public
            );
        }
    }
}

mod local_projection_window {
    use crate::support::model_identity as fixture_model_identity;
    use nola_agent_engine::agent::context_window::ContextWindowConfig;
    use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
    use nola_agent_engine::agent::{AgentSessionConfiguration, AgentThreadState, SessionId};
    use nola_application::{
        load_conversation_projection_v1, load_conversation_projection_window_v1,
    };
    use nola_domain::core_event::EventCursor;
    use nola_domain::{AssistantTurnBinding, PartMutation, TransitionMessageInput};
    use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore};
    use serde_json::json;
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };

    thread_local! {
        static TRACKING: Cell<bool> = const { Cell::new(false) };
        static MAXIMUM_ALLOCATION: Cell<usize> = const { Cell::new(0) };
        static LIVE_BYTES: Cell<usize> = const { Cell::new(0) };
        static PEAK_BYTES: Cell<usize> = const { Cell::new(0) };
    }
    struct MeasuredAllocator;
    unsafe impl GlobalAlloc for MeasuredAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                record(layout.size(), true);
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            record(layout.size(), false);
            unsafe { System.dealloc(pointer, layout) }
        }
        unsafe fn realloc(&self, pointer: *mut u8, old: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(pointer, old, size) };
            if !result.is_null() {
                record(old.size(), false);
                record(size, true);
            }
            result
        }
    }
    #[global_allocator]
    static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

    fn record(bytes: usize, allocating: bool) {
        if !TRACKING.try_with(Cell::get).unwrap_or(false) {
            return;
        }
        LIVE_BYTES.with(|live| {
            let next = if allocating {
                live.get().saturating_add(bytes)
            } else {
                live.get().saturating_sub(bytes)
            };
            live.set(next);
            if allocating {
                MAXIMUM_ALLOCATION.with(|maximum| maximum.set(maximum.get().max(bytes)));
                PEAK_BYTES.with(|peak| peak.set(peak.get().max(next)));
            }
        });
    }

    fn begin(store: &mut NolaStore, conversation: &str, message: &str, part: &str) {
        store
            .begin_turn(&BeginTurnInput {
                user_part_kind: "text".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                initial_events: Vec::new(),
                conversation_id: conversation.into(),
                conversation_title: "Canonical window fixture".into(),
                user_message_id: "fixture-user".into(),
                user_part_id: "fixture-user-part".into(),
                user_text: "Fixture input".into(),
                user_content_json: None,
                additional_user_parts: vec![],
                assistant_message_id: message.into(),
                assistant_part_id: part.into(),
                assistant_turn_bindings: vec![
                    AssistantTurnBinding::Admission {
                        turn_id: "shared-run".into(),
                        assistant_message_id: message.into(),
                    },
                    AssistantTurnBinding::Part {
                        turn_id: "shared-run".into(),
                        assistant_message_id: message.into(),
                        part_id: part.into(),
                    },
                ],
                initial_assistant_part_mutations: vec![],
                run_id: "shared-run".into(),
                context_checkpoint_id: None,
                checkpoint_revision: 0,
                checkpoint_event_cursor: 0,
                checkpoint_json: "{}".into(),
                invocation_config_json: "{}".into(),
                invocation_config_digest: "fixture".into(),
                outbox_effects: vec![],
                model_attempt_mutations: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                memory_usage_entries: vec![],
                created_at_ms: 1,
            })
            .unwrap();
    }
    fn transition(checkpoint: String) -> CommitTransitionInput {
        CommitTransitionInput {
            transition_id: "fixture-materialization".into(),
            run_id: "shared-run".into(),
            expected_checkpoint_revision: 0,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 1,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
                checkpoint_json: checkpoint,
            },
            submission_reduction_json: None,
            host_event: None,
            events: vec![],
            ensured_messages: vec![],
            part_message_owners: Default::default(),
            assistant_turn_bindings: vec![],
            part_mutations: vec![],
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![],
            memory_usage_entries: vec![],
            run_status: None,
            assistant_status: Some("completed".into()),
            assistant_status_message_id: None,
            committed_at_ms: 3,
        }
    }
    fn bind_part(input: &mut CommitTransitionInput, message: &str, part: &str, ordinal: i64) {
        input
            .part_message_owners
            .insert(part.into(), message.into());
        input
            .assistant_turn_bindings
            .push(AssistantTurnBinding::Part {
                turn_id: "shared-run".into(),
                assistant_message_id: message.into(),
                part_id: part.into(),
            });
        input.part_mutations.push(PartMutation::Upsert {
            part_id: part.into(),
            kind: "text".into(),
            ordinal,
            state: "completed".into(),
            content_text: String::new(),
            content_json: None,
            content_digest: None,
            created_at_ms: 2,
            updated_at_ms: 3,
        });
    }

    #[test]
    fn local_window_preserves_part_ownership_and_enforces_escaped_allocation_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("local-window.db");
        let mut state = AgentThreadState::new(
            SessionId::from_string("local-window".into()),
            AgentSessionConfiguration {
                model: None,
                max_model_iterations: 0,
                max_strategy_recovery_attempts: 3,
                tools: vec![],
                context_window: ContextWindowConfig {
                    context_window_tokens: 65_536,
                    reserved_output_tokens: 8_192,
                    ..ContextWindowConfig::default()
                },
                memory_policy: Default::default(),
                turn_identity: fixture_model_identity::identity(),
            },
        );
        state.conversation.context_window.used_tokens = 123;
        state.conversation.projection = serde_json::from_value(json!({
            "revision": 1,
            "active_message_segments": {"shared-run":"new"},
            "activity_message_owners": {},
            "order": ["old-part", "fallback-part", "new-part", "bound-part"],
            "parts": {
                "old-part": {"kind":"text", "id":"old-part", "message_id":"old",
                    "turn_id":"shared-run", "text":"x".repeat(10_000_000), "is_complete":true},
                "fallback-part": {"kind":"text", "id":"fallback-part", "message_id":"shared-run:assistant",
                    "turn_id":"shared-run", "text":"First message fallback", "is_complete":true},
                "new-part": {"kind":"text", "id":"new-part", "message_id":"new",
                    "turn_id":"shared-run", "text":"Latest message", "is_complete":true},
                "bound-part": {"kind":"text", "id":"bound-part", "message_id":"shared-run:assistant",
                    "turn_id":"shared-run", "text":"Explicit persisted Part binding", "is_complete":true}
            }
        }))
        .unwrap();
        let checkpoint = CoreCheckpoint {
            schema_version: nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: "local-window".into(),
            revision: 1,
            last_event_cursor: EventCursor { sequence: 1 },
            capability_catalog_revision: 0,
            created_at_ms: 3,
            reason: CheckpointReason::Manual,
            state,
        };
        let original_checkpoint = serde_json::to_string(&checkpoint).unwrap();
        {
            let mut store = NolaStore::open(&database).unwrap();
            begin(&mut store, "local-window", "old", "old-part");
            let mut input = transition(original_checkpoint.clone());
            input.ensured_messages.push(TransitionMessageInput {
                id: "new".into(),
                run_id: Some("shared-run".into()),
                role: "assistant".into(),
                status: "completed".into(),
                created_at_ms: 2,
                completed_at_ms: Some(3),
                initial_parts: vec![],
            });
            bind_part(&mut input, "old", "old-part", 1);
            bind_part(&mut input, "new", "new-part", 1);
            bind_part(&mut input, "new", "bound-part", 2);
            store.commit_transition(&input).unwrap();
        }
        drop(checkpoint);
        let store = NolaStore::open(&database).unwrap();
        let inputs = store
            .load_local_presentation_inputs("local-window", r#"["new"]"#, None)
            .unwrap()
            .unwrap();
        assert!(inputs.len() < 4096);
        let latest =
            load_conversation_projection_window_v1(&store, "local-window", None, 1, 4096).unwrap();
        assert_eq!(latest.total_message_count, 3);
        assert!(latest.has_more);
        assert_eq!(latest.projection.messages[0].message_id.0, "new");
        assert_eq!(latest.projection.context_usage.used_tokens, 0);
        assert!(latest.projection.context_usage.estimated);
        let parts = &latest.projection.messages[0].parts;
        assert!(
            matches!(&parts[0], nola_domain::MessagePartView::Text { part_id, markdown, .. }
                if part_id.0 == "new-part" && markdown == "Latest message")
        );
        assert!(
            matches!(&parts[1], nola_domain::MessagePartView::Text { part_id, markdown, .. }
                if part_id.0 == "bound-part" && markdown == "Explicit persisted Part binding")
        );
        MAXIMUM_ALLOCATION.with(|v| v.set(0));
        PEAK_BYTES.with(|v| v.set(0));
        LIVE_BYTES.with(|v| v.set(0));
        TRACKING.with(|v| v.set(true));
        let full = load_conversation_projection_v1(&store, "local-window");
        let older = load_conversation_projection_window_v1(
            &store,
            "local-window",
            latest.before_ordinal,
            1,
            4096,
        );
        TRACKING.with(|v| v.set(false));
        let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        assert!(full.unwrap_err().to_string().contains("bounded read budget"));
        assert!(older
            .unwrap_err()
            .to_string()
            .contains("bounded read budget"));
        assert!(maximum <= 32768, "maximum Rust allocation={maximum}");
        assert!(peak <= 65536, "peak Rust allocations={peak}");
        assert_eq!(
            store
                .load_checkpoint("local-window")
                .unwrap()
                .unwrap()
                .checkpoint_json,
            original_checkpoint
        );
        drop(store);

        // Escaped keys, values, and metadata bounds.
        for (case, id, text, session_id, admitted) in [
            (
                "bounded",
                "escaped\"\\\u{0001}界".to_string(),
                "\u{0001}\"\\界".repeat(20_000),
                "escape".to_string(),
                true,
            ),
            (
                "key",
                "\u{0001}".repeat(200_000),
                "body".to_string(),
                "escape".to_string(),
                false,
            ),
            (
                "value",
                "part".to_string(),
                "\u{0001}".repeat(400_000),
                "escape".to_string(),
                false,
            ),
            (
                "metadata",
                "part".to_string(),
                "body".to_string(),
                "\u{0001}".repeat(400_000),
                false,
            ),
        ] {
            let mut store = NolaStore::open(directory.path().join(format!("{case}.db"))).unwrap();
            begin(&mut store, "escape", "message", &id);
            let original = json!({"session_id": session_id, "state": {
                "session_id": "escape", "conversation": {"projection": {
                    "revision": 1, "order": [&id], "parts": {
                        (&id): {"kind": "text", "id": &id, "message_id": "message",
                            "turn_id": "shared-run", "text": &text, "is_complete": true}
                    }
                }}
            }})
            .to_string();
            let mut input = transition(original.clone());
            bind_part(&mut input, "message", &id, 1);
            store.commit_transition(&input).unwrap();
            MAXIMUM_ALLOCATION.with(|v| v.set(0));
            LIVE_BYTES.with(|v| v.set(0));
            PEAK_BYTES.with(|v| v.set(0));
            TRACKING.with(|v| v.set(true));
            let result = store.load_local_presentation_inputs("escape", r#"["message"]"#, None);
            TRACKING.with(|v| v.set(false));
            if admitted {
                let output = result.unwrap().unwrap();
                assert!(output.len() <= 2 * 1024 * 1024);
                let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
                assert_eq!(decoded["projection"]["order"][0], id);
                assert_eq!(decoded["projection"]["parts"][&id]["text"], text);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("bounded read budget"),
                    "{case}"
                );
                let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
                assert!(
                    maximum <= 32768,
                    "{case}: maximum Rust allocation={maximum}"
                );
            }
            assert_eq!(
                store.load_checkpoint("escape").unwrap().unwrap().checkpoint_json,
                original
            );
        }
    }
}

mod window_update_queries {
    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, CreateConversation,
        ExecuteReceivedCommand,
    };
    use nola_protocol::{
        ConversationTranscriptImportSource, ImportConversationTranscript,
        TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
    };

    fn update(id: &str, after_sequence: u64) -> ApplicationCommand {
        ApplicationCommand::LoadConversationProjectionWindowUpdateV1 {
            conversation_id: id.into(),
            before_ordinal: None,
            limit: 64,
            content_byte_limit: 16_384,
            after_sequence,
        }
    }

    #[test]
    fn receipted_transcript_import_and_rename_advance_window_updates_without_consuming_journal() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("window-updates.db");
        let mut runtime = support::open_runtime(&database).unwrap();
        for (id, title) in [("import-window", "History"), ("window", "Before")] {
            runtime
                .execute(ApplicationCommand::CreateConversation {
                    input: CreateConversation {
                        id: id.into(),
                        title: title.into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                })
                .unwrap();
        }
        assert!(runtime.execute(update("import-window", 0)).unwrap().is_null());
        assert!(runtime.execute(update("window", 0)).unwrap().is_null());
        assert!(runtime.execute(update("missing", 0)).is_err());
        assert!(runtime.execute(update("window", 1)).is_err());

        let import_cmd = ApplicationCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: "archive-1".into(),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "import-window".into(),
                expected_message_count: 0,
                imported_at_ms: 2,
                messages: vec![MessageInput {
                    id: "imported-message".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![MessagePartInput {
                        id: "imported-part".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "Imported evidence".into(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    }],
                }],
            },
        };
        let import_receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "import-client".into(),
                    request_id: "import-1".into(),
                    command_type: "import_conversation_transcript".into(),
                    command_json: serde_json::to_string(&import_cmd).unwrap(),
                    expected_revision: None,
                    received_at: "2026-09-08T00:00:00Z".into(),
                },
                import_cmd,
            )
            .unwrap();
        assert!(matches!(
            import_receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let evidence = runtime
            .execute(ApplicationCommand::SearchConversationText {
                conversation_id: "import-window".into(),
                query: "Imported evidence".into(),
                limit: 1,
            })
            .unwrap();
        assert_eq!(evidence.as_array().unwrap().len(), 1);
        let import_win = runtime.execute(update("import-window", 0)).unwrap();
        assert_eq!(
            import_win["projection"]["messages"][0]["message_id"],
            "imported-message"
        );
        let import_seq = import_win["event_sequence"].as_u64().unwrap();
        assert!(import_seq > 0);

        let rename_cmd = ApplicationCommand::RenameConversation {
            title_source: None,
            conversation_id: "window".into(),
            title: "After".into(),
            updated_at_ms: 2,
        };
        let rename_json = serde_json::to_string(&rename_cmd).unwrap();
        let rename_receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "window-client".into(),
                    request_id: "rename-window".into(),
                    command_type: "rename_conversation".into(),
                    command_json: rename_json,
                    expected_revision: None,
                    received_at: "2026-09-05T00:00:00Z".into(),
                },
                rename_cmd,
            )
            .unwrap();
        assert!(matches!(
            rename_receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let rename_win = runtime.execute(update("window", 0)).unwrap();
        assert_eq!(rename_win["projection"]["title"], "After");
        let rename_seq = rename_win["event_sequence"].as_u64().unwrap();
        assert!(rename_seq > 0);
        assert_eq!(runtime.execute(update("window", 0)).unwrap(), rename_win);
        assert!(runtime.execute(update("window", rename_seq)).unwrap().is_null());
        assert!(runtime.execute(update("window", rename_seq + 1)).is_err());

        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert_eq!(runtime.execute(update("import-window", 0)).unwrap(), import_win);
        assert!(runtime
            .execute(update("import-window", import_seq))
            .unwrap()
            .is_null());
        assert_eq!(runtime.execute(update("window", 0)).unwrap(), rename_win);
        assert!(runtime.execute(update("window", rename_seq)).unwrap().is_null());
    }
}
